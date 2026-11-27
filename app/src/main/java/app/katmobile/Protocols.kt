package app.katmobile

import kotlin.math.abs

data class Recognition(val name:String,val bits:Int,val evidence:String)

/** Passive, non-cryptographic format recognition. It never generates a transmission. */
class ProtocolRecognizer {
    private data class Candidate(val name:String,val bits:String,val repeats:Int,val evidence:String)
    private data class Seen(var count:Int,var at:Long)
    private val seen=linkedMapOf<String,Seen>()
    private fun close(value:Int,target:Double,tolerance:Double=.25)=abs(value-target)<=target*tolerance
    fun identify(pulses:List<Pulse>,modulation:String,frequency:Long=433_920_000,receiverPath:Boolean=false):Recognition? {
        NativeDecoder.identify(pulses,frequency,modulation,receiverPath)?.let{return it}
        if(modulation!="AM")return null
        val frames=mutableListOf<List<Pulse>>();var frame=mutableListOf<Pulse>()
        for(p in pulses){
            if(!p.level&&p.duration_us>=3000){if(frame.isNotEmpty())frames.add(frame);frame=mutableListOf()}
            else frame.add(p)
        }
        if(frame.isNotEmpty())frames.add(frame)
        var result:Recognition?=null
        for(raw in frames){
            val f=raw.dropWhile{!it.level}
            val c=princeton(f)?:fixed(f,"CAME",320.0)?:fixed(f,"Nice FLO",700.0)?:weather(f)?:continue
            val key=c.name+":"+c.bits
            val now=System.currentTimeMillis()
            val previous=seen[key]
            val entry=if(previous!=null&&now-previous.at<3000)previous.apply{count++;at=now}else Seen(1,now)
            seen[key]=entry
            while(seen.size>32)seen.remove(seen.keys.first())
            if(entry.count>=c.repeats)result=Recognition(c.name,c.bits.length,"${c.bits.length} bit · ${entry.count} matching frames · ${c.evidence}")
        }
        return result
    }
    private fun princeton(p:List<Pulse>):Candidate? {
        if(p.size!=49||!p.last().level)return null
        val te=p.map{it.duration_us}.sorted()[12].toDouble()
        if(te !in 200.0..600.0||!close(p.last().duration_us,te))return null
        val bits=StringBuilder()
        for(i in 0 until 48 step 2){val h=p[i];val l=p[i+1];if(!h.level||l.level)return null
            bits.append(when{close(h.duration_us,te)&&close(l.duration_us,te*3)->'0';close(h.duration_us,te*3)&&close(l.duration_us,te)->'1';else->return null})
        }
        if(bits.all{it=='0'}||bits.all{it=='1'})return null
        return Candidate("Princeton / EV1527",bits.toString(),2,"PWM 1:3 · family format, no CRC")
    }
    private fun fixed(p:List<Pulse>,name:String,te:Double):Candidate? {
        if(p.size !in listOf(25,49)||!p.first().level||!close(p.first().duration_us,te,.2))return null
        val bits=StringBuilder()
        for(i in 1 until p.size step 2){val l=p[i];val h=p[i+1];if(l.level||!h.level)return null
            bits.append(when{close(l.duration_us,te)&&close(h.duration_us,te*2)->'0';close(l.duration_us,te*2)&&close(h.duration_us,te)->'1';else->return null})
        }
        if(bits.all{it=='0'}||bits.all{it=='1'})return null
        return Candidate(name,bits.toString(),2,"PWM 1:2 · timing format, no CRC")
    }
    private fun weather(p:List<Pulse>):Candidate? {
        if(p.size !in listOf(72,73))return null
        if(p.size==73&&(!p.last().level||!close(p.last().duration_us,500.0)))return null
        val bits=StringBuilder()
        for(i in 0 until 72 step 2){val h=p[i];val l=p[i+1]
            if(!h.level||l.level||!close(h.duration_us,500.0))return null
            bits.append(when{close(l.duration_us,1000.0,.2)->'0';close(l.duration_us,2000.0,.2)->'1';else->return null})
        }
        if(bits.substring(24,28)!="1111"||bits.substring(10,12)=="11")return null
        val id=bits.substring(0,8).toInt(2);if(id==0||id==255)return null
        val tempBits=bits.substring(12,24).toInt(2);val temp=(if(tempBits and 0x800!=0)tempBits-4096 else tempBits)/10.0
        if(temp !in -60.0..80.0)return null
        val last=bits.substring(28,36).toInt(2)
        val bytes=listOf(bits.substring(0,8).toInt(2),bits.substring(8,16).toInt(2),bits.substring(16,24).toInt(2),0xf0,last)
        val crc=crc8(bytes)
        val channel=bits.substring(10,12).toInt(2)+1
        if(crc==0)return Candidate("Rubicson / Solight TE44",bits.toString(),2,"CRC8 OK · sensor $id · channel $channel · $temp °C")
        if(last>100)return null
        return Candidate("Nexus / TFA TH",bits.toString(),3,"constant F + repetitions · sensor $id · channel $channel · $temp °C · $last %")
    }
    private fun crc8(bytes:List<Int>):Int {
        var crc=0x6c
        for(b in bytes){crc=crc xor b;repeat(8){crc=if(crc and 128!=0)((crc shl 1)xor 0x31)and 255 else (crc shl 1)and 255}}
        return crc
    }
}
