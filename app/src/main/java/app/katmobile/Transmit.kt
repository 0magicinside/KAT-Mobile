package app.katmobile

/** Where the IQ goes: the HackRF's bulk OUT endpoint. [capacity] bytes can be queued before [write] waits for the HackRF. */
interface TxSink {
    val capacity:Int
    fun write(data:ByteArray,offset:Int,length:Int)
    /** Wait until the HackRF has taken everything written. */
    fun drain()
}

/**
 * One HackRF transmission, as desktop KAT's `transmit_signal_hackrf` sends it: 2 MS/s, a 1.75 MHz
 * baseband filter, the LO 500 kHz below the carrier (KAT's core shifts the waveform up by as much,
 * so the LO leakage stays out of the receiver's channel), the TX gain and the RF amplifier. The
 * first transfers are queued before the HackRF is keyed (libhackrf's order, so it never starts
 * empty), then the rest of the samples, then silence that pushes the last of them out of its 32 KB
 * buffer; then transmit mode goes off, also when anything fails.
 */
class HackRfTx(private val control:(Control)->Unit,private val sink:TxSink,private val pause:(Long)->Unit={Thread.sleep(it)}) {
    fun send(iq:ByteArray,carrierHz:Long,vgaGain:Int,amp:Boolean,cancelled:()->Boolean={false}){
        require(iq.isNotEmpty()&&iq.size%2==0){"No IQ samples to send"}
        val plan=Wire.txConfiguration(carrierHz,vgaGain,amp)
        var failure:Throwable?=null
        try{
            plan.forEach(control)
            val prime=minOf(iq.size,sink.capacity)
            var keyed=false
            var offset=0
            while(offset<iq.size){
                check(!cancelled()){"Transmission cancelled"}
                val n=minOf(CHUNK,iq.size-offset)
                sink.write(iq,offset,n)
                offset+=n
                if(!keyed&&offset>=prime){control(Wire.transceiver(Wire.MODE_TX));keyed=true}
            }
            val silence=ByteArray(CHUNK)
            var tail=0
            while(tail<TAIL){val n=minOf(CHUNK,TAIL-tail);sink.write(silence,0,n);tail+=n}
            sink.drain()
            pause(DRAIN_MS)
        }catch(t:Throwable){failure=t;throw t}
        finally{
            try{control(Wire.transceiver(Wire.MODE_OFF))}catch(t:Throwable){if(failure==null)throw t else failure.addSuppressed(t)}
        }
    }
    companion object {
        /** One USB transfer: 16 ms of samples at 2 MS/s. */
        const val CHUNK=65536
        /** Silence after the samples: twice the HackRF's 32 KB buffer. */
        const val TAIL=65536
        /** Desktop KAT's TX_DRAIN_MS: the HackRF's own buffer empties before it stops. */
        const val DRAIN_MS=25L
    }
}
