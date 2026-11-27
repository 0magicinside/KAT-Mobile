package app.katmobile

import com.google.gson.Gson
import com.google.gson.JsonObject
import com.google.gson.JsonParser

/** A button a capture can send: a car command (Lock 0x01, Unlock 0x02, Trunk 0x04, Panic 0x08) or a gate remote's own. */
data class TxButton(val code:Int=0,val label:String="")

/** What a capture can send besides Replay, as desktop KAT's signal menu offers it. */
data class TxActions(val encodes:Boolean=false,val gate:Boolean=false,val buttons:List<TxButton> = emptyList())

/** One transmission planned by KAT's core: what goes on air, its modulation, and for a rolling code the capture's decode after it (`sent`). */
data class TxPlan(val pulses:IntArray=intArrayOf(),val frequency:Long=0,val modulation:String="",val fsk:Boolean=false,
    val deviation_hz:Double=0.0,val preset:String="",val air_us:Long=0,val button:Int?=null,val button_label:String?=null,
    val sent:JsonObject?=null)

/** KAT's transmit side over JNI (native/src/mobile_tx.rs): what a capture can send, the plan of one transmission (Replay or KAT's encoders) and the HackRF's IQ. */
object NativeTx {
    private val gson=Gson()
    fun actions(decode:String):TxActions=
        if(decode.isBlank()||!NativeDecoder.available)TxActions()
        else runCatching{gson.fromJson(nativeActions(decode),TxActions::class.java)}.getOrDefault(TxActions())
    /** [action]: "replay" (the capture's pulses), "next" (the captured button's next code) or "button" ([button]'s next code). */
    fun plan(c:Capture,action:String,button:Int?=null):TxPlan {
        check(NativeDecoder.available){"The KAT core is unavailable"}
        val request=JsonObject().apply{
            addProperty("action",action)
            button?.let{addProperty("button",it)}
            if(c.decode.isNotBlank())add("decode",JsonParser.parseString(c.decode))
            if(action=="replay")add("pulses",gson.toJsonTree(c.pulses.map{if(it.level)it.duration_us else -it.duration_us}))
            addProperty("frequency",c.frequency)
            addProperty("received",c.modulation)
        }
        val out=JsonParser.parseString(nativePlan(gson.toJson(request))).asJsonObject
        if(!out["ok"].asBoolean)error(out["error"]?.asString?:"This capture cannot be sent")
        return gson.fromJson(out["plan"],TxPlan::class.java)
    }
    /** The HackRF's 8-bit IQ for [plan]: 2 MS/s, the waveform 500 kHz above the LO. */
    fun iq(plan:TxPlan):ByteArray=nativeIq(plan.pulses,plan.fsk,plan.deviation_hz)
    /** The decode a KAT .fob file holds ("" when it has none). */
    fun fobDecode(text:String):String=if(NativeDecoder.available)runCatching{nativeFobDecode(text)}.getOrDefault("")else ""
    @JvmStatic private external fun nativeActions(decode:String):String
    @JvmStatic private external fun nativePlan(request:String):String
    @JvmStatic private external fun nativeIq(pulses:IntArray,fsk:Boolean,deviationHz:Double):ByteArray
    @JvmStatic private external fun nativeFobDecode(text:String):String
}
