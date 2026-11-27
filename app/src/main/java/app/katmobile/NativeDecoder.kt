package app.katmobile

import com.google.gson.Gson

data class NativeProtocol(val name:String="",val rf:String="",val coding:String="",val format:String="")
private data class NativeRecognition(val name:String="",val bits:Int=0,val integrity:String="")

/** Read-only JNI API: protocol identity and integrity, without secret/authentication material. */
object NativeDecoder {
    private val gson=Gson()
    val protocols:List<NativeProtocol>
    val loadProblem:String
    val available get()=protocols.isNotEmpty()
    init {
        var problem=""
        protocols=try{
            System.loadLibrary("kat_rx")
            gson.fromJson(nativeCatalog(),Array<NativeProtocol>::class.java).toList().also{require(it.size>=100);nativeReset()}
        }catch(e:ExceptionInInitializerError){problem=e.message.orEmpty();emptyList()}
        catch(e:UnsatisfiedLinkError){problem=e.message.orEmpty();emptyList()}
        catch(e:Exception){problem=e.message.orEmpty();emptyList()}
        loadProblem=problem
    }
    fun identify(pulses:List<Pulse>,frequency:Long,modulation:String,receiverPath:Boolean=false):Recognition? {
        if(!available||pulses.isEmpty()||pulses.size>32768)return null
        val input=IntArray(pulses.size){i->val p=pulses[i];if(p.level)p.duration_us else -p.duration_us}
        return runCatching{
            val found=gson.fromJson(nativeIdentify(input,frequency,modulation=="FM",receiverPath),Array<NativeRecognition>::class.java)
                .filter{it.name.isNotBlank()&&it.bits in 1..4096}
            if(found.isEmpty())null else Recognition(found.joinToString(" + "){it.name},found.first().bits,
                "KAT RX · "+found.joinToString("; "){"${it.name}: ${it.bits} bit · ${it.integrity}"})
        }.getOrNull()
    }
    fun reset(){if(available)nativeReset()}
    @JvmStatic private external fun nativeCatalog():String
    @JvmStatic private external fun nativeIdentify(pulses:IntArray,frequency:Long,fm:Boolean,receiverPath:Boolean):String
    @JvmStatic private external fun nativeReset()
}
