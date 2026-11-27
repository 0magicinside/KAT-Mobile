package app.katmobile

import android.hardware.usb.*
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * What the embedded RTL-SDR Rust driver asks of the dongle: native/src/mobile_rtlsdr.rs calls these
 * two methods by name and signature over JNI. [RtlControlBridge] is the dongle; the device tests
 * stand a simulated one in its place.
 */
internal interface RtlUsbControl {
    /** A USB control transfer, as `UsbDeviceConnection.controlTransfer`: bytes moved, or negative. */
    fun controlTransfer(requestType:Int,request:Int,value:Int,index:Int,data:ByteArray,length:Int,timeoutMs:Int):Int
    /** Claim the interface again (librtlsdr resets the dongle when its first write fails). */
    fun resetInterface():Boolean
}

/** Control transfer callback invoked by the embedded MPL-2.0 RTL-SDR Rust driver over JNI. */
internal class RtlControlBridge(
    private val connection:UsbDeviceConnection,
    private val iface:UsbInterface
):RtlUsbControl {
    /** Called from native/src/mobile_rtlsdr.rs; never throws across JNI. */
    override fun controlTransfer(
        requestType:Int,
        request:Int,
        value:Int,
        index:Int,
        data:ByteArray,
        length:Int,
        timeoutMs:Int
    ):Int = try {
        connection.controlTransfer(requestType,request,value,index,data,length,timeoutMs)
    } catch (_:Exception) {
        -1
    }

    /** Soft interface re-claim used if the initial dummy write stalls. */
    override fun resetInterface():Boolean = try {
        connection.releaseInterface(iface)
        connection.claimInterface(iface,true)
    } catch (_:Exception) {
        false
    }
}

/** JNI bindings to native/src/mobile_rtlsdr.rs (RTL2832U + R820T/R820T2/R828D, incl. RTL-SDR Blog V3/V4). */
internal object NativeRtlSdr {
    fun open(bridge:RtlUsbControl,manufacturer:String,product:String,serial:String):Long {
        check(NativeDecoder.available){"The native KAT RTL-SDR driver is unavailable"}
        return nativeOpen(bridge,manufacturer,product,serial)
    }
    fun tunerName(handle:Long):String=nativeTunerName(handle)
    fun tunerGains(handle:Long):IntArray=nativeTunerGains(handle)
    fun configure(handle:Long,centerHz:Long,sampleRate:Int,autoGain:Boolean,gainTenthsDb:Int,ppm:Int,biasTee:Boolean)=
        nativeConfigure(handle,centerHz,sampleRate,autoGain,gainTenthsDb,ppm,biasTee)
    fun setCenterFreq(handle:Long,centerHz:Long)=nativeSetCenterFreq(handle,centerHz)
    fun resetBuffer(handle:Long)=nativeResetBuffer(handle)
    fun close(handle:Long)=nativeClose(handle)

    @JvmStatic private external fun nativeOpen(bridge:Any,manufacturer:String,product:String,serial:String):Long
    @JvmStatic private external fun nativeTunerName(handle:Long):String
    @JvmStatic private external fun nativeTunerGains(handle:Long):IntArray
    @JvmStatic private external fun nativeConfigure(handle:Long,centerHz:Long,sampleRate:Int,autoGain:Boolean,gainTenthsDb:Int,ppm:Int,biasTee:Boolean)
    @JvmStatic private external fun nativeSetCenterFreq(handle:Long,centerHz:Long)
    @JvmStatic private external fun nativeResetBuffer(handle:Long)
    @JvmStatic private external fun nativeClose(handle:Long)
}

/**
 * Receive-only RTL-SDR driver over Android's USB Host API (no root, no external driver app).
 * Control transfers (RTL2832U demodulator + Rafael Micro R820T/R820T2/R828D tuner) run through
 * KAT's embedded MPL-2.0 Rust driver (`NativeRtlSdr`); high-throughput bulk IN (0x81) CU8 IQ
 * streaming uses Android's asynchronous `UsbRequest` queue.
 */
class RtlSdrUsb(private val manager:UsbManager,private val device:UsbDevice):SdrUsb {
    override val kind=RadioKind.RTLSDR
    override val iqFormat=IqFormat.CU8

    private val iface=(0 until device.interfaceCount).map(device::getInterface).firstOrNull{i->
        (0 until i.endpointCount).any{e->val ep=i.getEndpoint(e);ep.address==0x81&&ep.type==UsbConstants.USB_ENDPOINT_XFER_BULK}
    }?:error("No RTL-SDR bulk RX interface found")
    private val endpoint=(0 until iface.endpointCount).map(iface::getEndpoint)
        .first{it.address==0x81&&it.type==UsbConstants.USB_ENDPOINT_XFER_BULK}
    private val connection=manager.openDevice(device)?:error("Android could not open the RTL-SDR USB device")
    private val bridge=RtlControlBridge(connection,iface)
    private val controlLock=Any()
    private val handle=AtomicLong(0L)
    private val stopped=AtomicBoolean(false)
    private val closed=AtomicBoolean(false)
    private val queued=ArrayList<UsbRequest>()
    @Volatile private var tuneGeneration=0L
    @Volatile private var activeSampleRate=2_000_000
    override val description:String

    init {
        try {
            check(connection.claimInterface(iface,true)){"The RTL-SDR USB interface is busy"}
            val manufacturer=runCatching{device.manufacturerName.orEmpty()}.getOrDefault("")
            val product=runCatching{device.productName.orEmpty()}.getOrDefault("")
            val serial=runCatching{device.serialNumber.orEmpty()}.getOrDefault("")
            val h=NativeRtlSdr.open(bridge,manufacturer,product,serial)
            check(h!=0L){"Could not initialize RTL-SDR"}
            handle.set(h)
            description=runCatching{NativeRtlSdr.tunerName(h)}.getOrDefault("RTL2832U")
        } catch (e:Exception) {
            close()
            throw e
        }
    }

    override fun sampleRate(s:RadioSettings):Int=s.rtlSampleRate

    override fun configure(s:RadioSettings,center:Long) {
        s.validate()
        synchronized(controlLock) {
            val h=requireOpen()
            activeSampleRate=s.rtlSampleRate
            NativeRtlSdr.configure(h,center,s.rtlSampleRate,s.rtlAutoGain,s.rtlGainTenthsDb,s.rtlPpm,s.rtlBiasTee)
        }
    }

    override fun tune(hz:Long) {
        synchronized(controlLock) {
            val h=requireOpen()
            NativeRtlSdr.setCenterFreq(h,hz)
            tuneGeneration++
        }
    }

    override fun isCurrentGeneration(generation:Long)=generation==tuneGeneration

    override fun read(hopping:Boolean,consume:(ByteArray,Int,Long)->Unit) {
        val buffers=ArrayList<ByteBuffer>()
        // 64 KiB (~16 ms at 2 MS/s) matches desktop KAT; 16 KiB keeps hop latency low and works on API 26–27.
        val capacity=if(!hopping&&android.os.Build.VERSION.SDK_INT>=28)65536 else 16384
        val generations=HashMap<UsbRequest,Long>()
        try {
            synchronized(controlLock) {
                val h=requireOpen()
                NativeRtlSdr.resetBuffer(h)
            }
            repeat(8) {
                val request=UsbRequest()
                check(request.initialize(connection,endpoint)){"Could not open the RTL-SDR USB RX queue"}
                val buffer=ByteBuffer.allocateDirect(capacity)
                request.clientData=buffer
                buffers.add(buffer)
                synchronized(queued){queued.add(request)}
                generations[request]=tuneGeneration
                check(request.queue(buffer)){"The RTL-SDR USB buffer was rejected"}
            }
            val bytes=ByteArray(capacity)
            var last=android.os.SystemClock.elapsedRealtime()
            var seenGeneration=tuneGeneration
            var discardRemaining=0
            while(!stopped.get()) {
                val request=try{connection.requestWait(500)}catch(_:java.util.concurrent.TimeoutException){null}
                if(stopped.get())break
                if(request==null) {
                    if(android.os.SystemClock.elapsedRealtime()-last>5000) {
                        error("No USB data from RTL-SDR. Check the OTG cable and power supply.")
                    }
                    continue
                }
                val buffer=request.clientData as ByteBuffer
                val rawLen=buffer.position()
                val gen=generations.getValue(request)
                if(rawLen>0&&gen==tuneGeneration) {
                    last=android.os.SystemClock.elapsedRealtime()
                    if(gen!=seenGeneration) {
                        seenGeneration=gen
                        discardRemaining=retuneFlushBytes(activeSampleRate)
                    }
                    val even=rawLen and -2
                    if(even>0) {
                        if(discardRemaining>0) {
                            discardRemaining=(discardRemaining-even).coerceAtLeast(0)
                        } else {
                            buffer.flip()
                            buffer.get(bytes,0,even)
                            consume(bytes,even,gen)
                        }
                    }
                }
                buffer.clear()
                generations[request]=tuneGeneration
                if(!stopped.get())check(request.queue(buffer)){"RTL-SDR USB RX transfer interrupted"}
            }
        } finally {
            synchronized(queued) {
                queued.forEach{runCatching{it.cancel()};runCatching{it.close()}}
                queued.clear()
            }
        }
    }

    override fun stop() {
        if(stopped.compareAndSet(false,true)) {
            synchronized(queued){queued.forEach{runCatching{it.cancel()}}}
        }
    }

    override fun close() {
        stop()
        if(closed.compareAndSet(false,true)) {
            synchronized(controlLock) {
                val h=handle.getAndSet(0L)
                if(h!=0L)runCatching{NativeRtlSdr.close(h)}
                runCatching{connection.releaseInterface(iface)}
                runCatching{connection.close()}
            }
        }
    }

    private fun requireOpen():Long {
        val h=handle.get()
        check(!stopped.get()&&h!=0L){"The RTL-SDR USB connection is closed"}
        return h
    }

    companion object {
        /** Supported RTL-SDR sample rates (Hz); 2.0 MS/s matches desktop KAT's RTL-SDR path. */
        val SAMPLE_RATES=listOf(1_024_000,2_000_000,2_048_000,2_400_000)

        /** Discrete Rafael Micro R820T/R820T2/R828D tuner gain steps in tenths of dB. */
        val R82XX_GAINS_TENTHS_DB=listOf(
            0,9,14,27,37,77,87,125,144,157,166,197,207,229,254,
            280,297,328,338,364,372,386,402,421,434,439,445,480,496
        )

        /** ~4 ms of CU8 IQ discarded after a retune (16 KiB at 2 MS/s, matching desktop KAT's RETUNE_FLUSH_LEN). */
        fun retuneFlushBytes(sampleRate:Int):Int=((sampleRate.toLong()*2L*4L)/1000L).toInt().coerceAtLeast(8192) and -2

        fun gainLabel(auto:Boolean,tenthsDb:Int):String=
            if(auto)"Auto (AGC)" else "%.1f dB".format(java.util.Locale.US,tenthsDb/10.0)
    }
}
