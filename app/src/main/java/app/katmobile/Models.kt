package app.katmobile

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID

data class RadioSettings(
    val frequency:Long=433_920_000, val sampleRate:Int=2_000_000,
    val lna:Int=32, val vga:Int=32, val amp:Boolean=false,
    val bandwidth:Int=1_750_000, val modulation:String="AUTO",
    val hopping:Boolean=false, val hopFrequencies:String="315,390,430.50,433.92,434.25,434.42,868.35",
    val hopDwellMs:Int=1000, val thresholdDb:Int=10, val minimumPulseUs:Int=20,
    val burstGapUs:Int=20000, val maxCaptures:Int=500,
    val showUnknown:Boolean=true, val ignoreDuplicates:Boolean=true,
    val autosave:Boolean=true, val includeRaw:Boolean=true, val recordingQuotaMb:Int=1024,
    val fontSize:Int=13, val keepScreen:Boolean=true
) {
    fun frequencies()=hopFrequencies.split(',').filter{it.isNotBlank()}.map{mhz(it)}.also{require(it.size in 1..32)}
    fun validate(){
        require(frequency in 1_000_000L..6_000_000_000L){"Frequency must be 1–6000 MHz"}
        require(sampleRate in listOf(2_000_000,4_000_000,8_000_000,10_000_000,16_000_000,20_000_000)){"Unsupported sample rate"}
        require(lna in 0..40&&lna%8==0&&vga in 0..62&&vga%2==0){"LNA: 0–40 / 8 dB; VGA: 0–62 / 2 dB"}
        require(bandwidth in Wire.bandwidths&&bandwidth<=sampleRate){"Choose a filter no wider than the sample rate"}
        require(modulation in listOf("AUTO","AM","FM"))
        frequencies();require(hopDwellMs in 250..30000)
        require(thresholdDb in 3..40&&minimumPulseUs in 4..1000&&burstGapUs in 1000..200000)
        require(maxCaptures in 10..2000&&recordingQuotaMb in 100..100000&&fontSize in 10..22)
    }
    companion object {fun mhz(s:String):Long=s.trim().replace(',','.').toBigDecimal().multiply(1_000_000.toBigDecimal()).longValueExact().also{require(it in 1_000_000L..6_000_000_000L)}}
}
data class Pulse(val level:Boolean=false,val duration_us:Int=0)
data class Capture(
    val id:String=UUID.randomUUID().toString(),val time:Long=System.currentTimeMillis(),
    val frequency:Long=433_920_000,val modulation:String="AM",val protocol:String="Unknown",
    val source:String="HackRF",val pulses:List<Pulse> = emptyList(),val repeats:Int=1,
    val serial:String="",val button:String="",val counter:String="",val metadata:String="",val recognition:String=""
){val durationUs get()=pulses.sumOf{it.duration_us.toLong()};val fingerprint get()=pulses.dropWhile{!it.level}.dropLastWhile{!it.level&&it.duration_us>=3000}.take(256).map{(if(it.level)1 else -1)*(it.duration_us/20)}.hashCode()}
data class Saved(val settings:RadioSettings=RadioSettings(),val captures:List<Capture> = emptyList())
data class IqSpec(val rate:Int,val center:Long,val target:Long){val offsetHz get()=(target-center).toDouble()}
data class RadioStatus(val running:Boolean=false,val device:String="No device",val firmware:String="",
    val message:String="Connect HackRF using USB OTG",val frequency:Long=433_920_000,
    val hopIndex:Int=0,val hopTotal:Int=0,val hopSwitches:Long=0,
    val bytes:Long=0,val bytesPerSecond:Long=0,val gaps:Long=0,val powerDb:Float=-100f,
    val spectrum:List<Float> = emptyList(),val recording:String="",val recordingBytes:Long=0)

object StateFormat {
    val gson=Gson()
    private fun merge(default:JsonObject,incoming:JsonObject):JsonObject=default.deepCopy().also{out->incoming.entrySet().forEach{(k,v)->if(!v.isJsonNull)out.add(k,if(v.isJsonObject&&out[k]?.isJsonObject==true)merge(out[k].asJsonObject,v.asJsonObject)else v)}}
    fun decode(text:String):Saved {
        require(text.length<32*1024*1024)
        val root=merge(gson.toJsonTree(Saved()).asJsonObject,JsonParser.parseString(text).asJsonObject)
        root.add("settings",merge(gson.toJsonTree(RadioSettings()).asJsonObject,root["settings"].asJsonObject))
        val items=com.google.gson.JsonArray()
        root["captures"].asJsonArray.forEach{items.add(merge(gson.toJsonTree(Capture()).asJsonObject,it.asJsonObject))}
        root.add("captures",items)
        val state=gson.fromJson(root,Saved::class.java);state.settings.validate()
        require(state.captures.size<=2000)
        state.captures.forEach{require(it.pulses.size<=32768&&it.pulses.all{p->p.duration_us in 1..60_000_000})}
        return state
    }
    fun importFob(text:String):Capture {
        require(text.length<=4*1024*1024)
        val root=JsonParser.parseString(text).asJsonObject
        val c=if(root["capture"]?.isJsonObject==true)root["capture"].asJsonObject else root
        val signal=root["signal"]?.takeIf{it.isJsonObject}?.asJsonObject
        fun str(k:String)=(signal?.get(k)?:c[k])?.takeIf{it.isJsonPrimitive}?.asString.orEmpty()
        val pulses=(c["raw_pairs"]?:c["pulses"])?.takeIf{it.isJsonArray}?.asJsonArray?.map{val p=it.asJsonObject;Pulse(p["level"].asBoolean,p["duration_us"].asInt)}.orEmpty()
        require(pulses.size<=32768&&pulses.all{it.duration_us in 1..60_000_000})
        val recordedTime=str("time").toLongOrNull()?:runCatching{java.time.Instant.parse(str("timestamp")).toEpochMilli()}.getOrNull()?:System.currentTimeMillis()
        return Capture(time=recordedTime,frequency=str("frequency").toLongOrNull()?:433_920_000,protocol=str("protocol").ifBlank{"Imported"},
            modulation=str("received_rf").ifBlank{str("modulation")}.ifBlank{"AM"}.uppercase(java.util.Locale.US),source="FOB import",pulses=pulses,
            serial=str("serial"),button=str("button"),counter=str("counter"),metadata=text)
    }
    /** KAT v2 interchange. A waveform export always carries every saved pair. */
    fun fob(c:Capture):String {
        require(c.pulses.isNotEmpty()){"No saved pulses for FOB export"}
        val existing=runCatching{JsonParser.parseString(c.metadata).asJsonObject}.getOrNull()
        val root=if(existing?.get("format")?.asString=="kat-fob"&&existing.get("signal")?.isJsonObject==true)existing.deepCopy()else JsonObject().apply{
            addProperty("version","2.0");addProperty("format","kat-fob")
            add("signal",JsonObject().apply{
                addProperty("protocol",c.protocol);addProperty("frequency",c.frequency)
                addProperty("frequency_mhz",java.lang.String.format(java.util.Locale.US,"%.3f",c.frequency/1e6))
                addProperty("modulation","Unknown");addProperty("received_rf",c.modulation)
                addProperty("encryption","Unknown");addProperty("data_bits",0);addProperty("data_hex","")
                addProperty("serial","-");addProperty("key","");addProperty("button_name","-")
                addProperty("crc_valid",false);addProperty("encoder_capable",false)
                if(c.modulation=="AM")addProperty("preset","FuriHalSubGhzPresetOok650Async")
            })
            add("vehicle",JsonObject().apply{addProperty("make","")})
            add("capture",JsonObject().apply{addProperty("timestamp",java.time.Instant.ofEpochMilli(c.time).toString())})
        }
        root.getAsJsonObject("capture").apply{add("raw_pairs",gson.toJsonTree(c.pulses));addProperty("raw_pair_count",c.pulses.size)}
        return gson.toJson(root)
    }
    fun iqSpec(name:String,s:RadioSettings):IqSpec {
        val rate=Regex("_(\\d+)sps",RegexOption.IGNORE_CASE).find(name)?.groupValues?.get(1)?.toIntOrNull()?:s.sampleRate
        val center=Regex("_(\\d+)Hz",RegexOption.IGNORE_CASE).find(name)?.groupValues?.get(1)?.toLongOrNull()?:s.frequency
        val target=Regex("_rx(\\d+)Hz",RegexOption.IGNORE_CASE).find(name)?.groupValues?.get(1)?.toLongOrNull()?:center
        require(rate in 250_000..20_000_000&&center in 0..6_000_000_000L&&target in 1_000_000L..6_000_000_000L)
        require(kotlin.math.abs(target-center)<=rate/2){"Target frequency is outside the IQ recording bandwidth"}
        return IqSpec(rate,center,target)
    }
    fun importRawSub(text:String):Capture {
        require(text.length<=4*1024*1024)
        val frequency=Regex("(?m)^Frequency:\\s*(\\d+)").find(text)?.groupValues?.get(1)?.toLong()?:error("Missing frequency")
        require(Regex("(?m)^Protocol:\\s*RAW\\s*$").containsMatchIn(text)){"Analysis supports RAW .sub files"}
        val values=text.lineSequence().filter{it.startsWith("RAW_Data:")}.flatMap{it.substringAfter(':').trim().split(Regex("\\s+")).asSequence()}.map{it.toLong()}.toList()
        require(values.size in 2..32768&&values.all{it in -60_000_000L..60_000_000L&&it!=0L})
        val preset=Regex("(?m)^Preset:\\s*(.*)$").find(text)?.groupValues?.get(1).orEmpty()
        val modulation=if(preset.contains("FSK",true)||preset.contains("FM",true))"FM"else"AM"
        val c=Capture(frequency=frequency,modulation=modulation,source="RAW import",pulses=values.map{Pulse(it>0,kotlin.math.abs(it).toInt())})
        val match=ProtocolRecognizer().identify(c.pulses,c.modulation,c.frequency)
        return if(match==null)c else c.copy(protocol=match.name,recognition=match.evidence)
    }
    fun rawSub(c:Capture):String="Filetype: Flipper SubGhz RAW File\nVersion: 1\nFrequency: ${c.frequency}\nPreset: FuriHalSubGhzPresetOok650Async\nProtocol: RAW\n"+
        c.pulses.chunked(512).joinToString("\n"){batch->"RAW_Data: "+batch.joinToString(" "){((if(it.level)1 else -1)*it.duration_us).toString()}}+"\n"
    fun captureJson(c:Capture,includeRaw:Boolean):String {
        val root=if(c.metadata.isNotBlank())JsonParser.parseString(c.metadata).asJsonObject else gson.toJsonTree(c).asJsonObject
        if(!includeRaw){val target=if(root["capture"]?.isJsonObject==true)root["capture"].asJsonObject else root;target.remove("raw_pairs");target.remove("pulses")}
        return gson.toJson(root)
    }
    fun ini(text:String,base:RadioSettings):RadioSettings {
        require(text.length<=128*1024)
        val values=text.lineSequence().map{it.substringBefore(';').substringBefore('#').trim()}.filter{'=' in it}.associate{it.substringBefore('=').trim() to it.substringAfter('=').trim()}
        fun i(k:String,d:Int)=values[k]?.toIntOrNull()?:d
        fun b(k:String,d:Boolean)=values[k]?.lowercase()?.let{it in listOf("true","yes","1","on")}?:d
        return base.copy(frequency=values["default_frequency"]?.toLongOrNull()?:base.frequency,
            lna=i("default_lna_gain",base.lna),vga=i("default_vga_gain",base.vga),amp=b("default_amp",base.amp),
            hopFrequencies=values["hop_frequencies"]?.split(',')?.joinToString(","){(it.trim().toBigDecimal()/1_000_000.toBigDecimal()).toPlainString()}?:base.hopFrequencies,
            hopDwellMs=i("hop_dwell_ms",base.hopDwellMs),maxCaptures=i("max_captures",base.maxCaptures).let{if(it==0)2000 else it},
            showUnknown=b("show_unknown_signals",base.showUnknown),ignoreDuplicates=b("ignore_duplicates",base.ignoreDuplicates),
            autosave=b("session_autosave",base.autosave),includeRaw=b("include_raw_pairs",base.includeRaw)).also{it.validate()}
    }
}

data class Control(val input:Boolean,val request:Int,val value:Int=0,val index:Int=0,val data:ByteArray=byteArrayOf())
object Wire {
    fun liveCenter(frequency:Long,rate:Int)=(frequency-rate/8L).coerceAtLeast(0L)
    val bandwidths=listOf(1_750_000,2_500_000,3_500_000,5_000_000,5_500_000,6_000_000,7_000_000,8_000_000,9_000_000,10_000_000,12_000_000,14_000_000,15_000_000,20_000_000)
    private fun pair(a:Int,b:Int)=ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putInt(a).putInt(b).array()
    fun frequency(hz:Long)=Control(false,16,data=pair((hz/1_000_000).toInt(),(hz%1_000_000).toInt()))
    fun configuration(s:RadioSettings):List<Control>{s.validate();return listOf(Control(false,1),Control(false,6,data=pair(s.sampleRate,1)),
        Control(false,7,s.bandwidth and 65535,s.bandwidth ushr 16),frequency(s.frequency),
        Control(true,19,index=s.lna,data=ByteArray(1)),Control(true,20,index=s.vga,data=ByteArray(1)),Control(false,17,if(s.amp)1 else 0))}
}
