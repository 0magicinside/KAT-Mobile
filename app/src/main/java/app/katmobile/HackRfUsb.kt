package app.katmobile

import android.hardware.usb.*
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean

/** Receive-only implementation of the published HackRF vendor protocol. */
class HackRfUsb(private val manager:UsbManager,private val device:UsbDevice):AutoCloseable {
    private val iface=(0 until device.interfaceCount).map(device::getInterface).firstOrNull{i->(0 until i.endpointCount).any{i.getEndpoint(it).address==0x81}}?:error("No HackRF RX interface")
    private val endpoint=(0 until iface.endpointCount).map(iface::getEndpoint).first{it.address==0x81&&it.type==UsbConstants.USB_ENDPOINT_XFER_BULK}
    private val connection=manager.openDevice(device)?:error("Android could not open the USB device")
    private val stopped=AtomicBoolean(false)
    private val queued=ArrayList<UsbRequest>()
    @Volatile private var tuneGeneration=0L
    val description:String
    init {
        try{
            check(connection.claimInterface(iface,true)){"The USB interface is busy"}
            val version=ByteArray(128);val n=connection.controlTransfer(0xC0,15,0,0,version,version.size,1000)
            check(n>0){"HackRF did not respond to the firmware request"}
            description=String(version,0,n).trimEnd('\u0000')
        }catch(e:Exception){connection.close();throw e}
    }
    @Synchronized fun control(c:Control){
        check(!stopped.get()){"The USB connection is closed"}
        val result=connection.controlTransfer(if(c.input)0xC0 else 0x40,c.request,c.value,c.index,c.data,c.data.size,1000)
        check(result==c.data.size&&(!c.input||c.request !in listOf(19,20)||c.data[0].toInt()!=0)){"HackRF request ${c.request} rejected ($result)"}
    }
    fun configure(s:RadioSettings,center:Long=s.frequency){Wire.configuration(s).map{if(it.request==16)Wire.frequency(center)else it}.forEach(::control)}
    fun tune(hz:Long){control(Wire.frequency(hz));tuneGeneration++}
    fun isCurrentGeneration(generation:Long)=generation==tuneGeneration
    fun read(hopping:Boolean=false,consume:(ByteArray,Int,Long)->Unit){
        val buffers=ArrayList<ByteBuffer>()
        // Short queues keep a hop from spending its dwell on old-frequency samples.
        val capacity=if(!hopping&&android.os.Build.VERSION.SDK_INT>=28)262144 else 16384
        val generations=HashMap<UsbRequest,Long>()
        try{
            repeat(8){val request=UsbRequest();check(request.initialize(connection,endpoint)){"Could not open the USB RX queue"};val buffer=ByteBuffer.allocateDirect(capacity);request.clientData=buffer;buffers.add(buffer);queued.add(request);generations[request]=tuneGeneration;check(request.queue(buffer)){"The USB buffer was rejected"}}
            control(Control(false,1,value=1))
            val bytes=ByteArray(capacity);var last=android.os.SystemClock.elapsedRealtime()
            while(!stopped.get()){
                val request=try{connection.requestWait(1000)}catch(_:java.util.concurrent.TimeoutException){null}
                if(request==null){if(android.os.SystemClock.elapsedRealtime()-last>5000)error("No USB data. Check the OTG cable and power supply.");continue}
                val buffer=request.clientData as ByteBuffer;val length=buffer.position()
                if(length>0&&generations[request]==tuneGeneration){require(length%2==0){"Incomplete IQ sample"};buffer.flip();buffer.get(bytes,0,length);consume(bytes,length,generations.getValue(request));last=android.os.SystemClock.elapsedRealtime()}
                buffer.clear();generations[request]=tuneGeneration;if(!stopped.get())check(request.queue(buffer)){"USB RX transfer interrupted"}
            }
        }finally{
            runCatching{if(!stopped.get())control(Control(false,1))}
            queued.forEach{runCatching{it.cancel()};runCatching{it.close()}};queued.clear()
        }
    }
    fun stop(){if(stopped.compareAndSet(false,true)){runCatching{connection.controlTransfer(0x40,1,0,0,null,0,500)};connection.close()}}
    override fun close(){stop()}
    companion object {
        fun supported(d:UsbDevice)=d.vendorId==0x1d50&&d.productId in listOf(0x6089,0x604b,0xcc15)
        fun devices(manager:UsbManager)=manager.deviceList.values.filter(::supported).sortedBy{it.deviceName}
    }
}
