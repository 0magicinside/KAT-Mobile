package app.katmobile

import android.app.*
import android.content.*
import android.hardware.usb.UsbManager
import android.os.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.update
import java.io.BufferedOutputStream
import java.io.File

class RadioService:Service() {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.IO)
    private var job:Job?=null
    @Volatile private var radio:HackRfUsb?=null
    private var wake:PowerManager.WakeLock?=null
    private val fileLock=Any()
    private var recording:BufferedOutputStream?=null
    private var recordFile:File?=null
    private var recorded=0L
    private var recordLimit=0L
    private var lastFrequency=0L
    private var bytes=0L
    override fun onBind(intent:Intent?)=null
    override fun onCreate(){super.onCreate();Hub.initialize(this);getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("radio","USB radio reception",NotificationManager.IMPORTANCE_LOW))}
    override fun onStartCommand(intent:Intent?,flags:Int,startId:Int):Int {
        if(intent?.action=="START"){
            val manager=getSystemService(UsbManager::class.java)
            val device=manager.deviceList.values.firstOrNull{it.deviceName==intent.getStringExtra("device")&&HackRfUsb.supported(it)}
            if(device==null||!manager.hasPermission(device)){Hub.notice.value="The USB device is no longer available";stopSelf();return START_NOT_STICKY}
            try{startForeground(8,notification())}catch(e:Exception){Hub.notice.value=e.message?:"Reception did not start";stopSelf();return START_NOT_STICKY}
        }else if(job?.isActive!=true){stopSelf();return START_NOT_STICKY}
        when(intent?.action){
            "START"->if(job?.isActive!=true)startReceiver(intent.getStringExtra("device").orEmpty())
            "RECORD"->toggleRecording()
            else->{radio?.stop();job?.cancel();closeRecording();stopSelf()}
        }
        return START_NOT_STICKY
    }
    private fun notification():Notification {
        val open=PendingIntent.getActivity(this,0,Intent(this,MainActivity::class.java),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop=PendingIntent.getService(this,1,Intent(this,RadioService::class.java).setAction("STOP"),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this,"radio").setSmallIcon(R.drawable.ic_radio).setContentTitle("KAT Mobile · RX")
            .setContentText("${"%.3f".format(Hub.status.value.frequency/1e6)} MHz · USB reception")
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null,"Stop",stop).build()).build()
    }
    private fun startReceiver(name:String){
        val manager=getSystemService(UsbManager::class.java)
        val device=manager.deviceList.values.firstOrNull{it.deviceName==name&&HackRfUsb.supported(it)}
        if(device==null||!manager.hasPermission(device)){Hub.notice.value="HackRF is unavailable or USB permission is missing";stopSelf();return}
        val s=Hub.settings.value
        Hub.status.value=RadioStatus(running=true,device=device.productName?:"HackRF",frequency=s.frequency,message="Opening USB reception…")
        wake=getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"KAT:rx").apply{acquire(24*60*60*1000L)}
        job=scope.launch {
            var engine:ReceivePipeline?=null
            try{
                HackRfUsb(manager,device).use{usb->
                    radio=usb
                    val hopFrequencies=if(s.hopping)s.frequencies()else emptyList()
                    lastFrequency=hopFrequencies.firstOrNull()?:s.frequency
                    val center=Wire.liveCenter(lastFrequency,s.sampleRate);usb.configure(s,center)
                    engine=ReceivePipeline(s.copy(frequency=lastFrequency),(lastFrequency-center).toDouble(),Hub::add)
                    Hub.status.value=Hub.status.value.copy(firmware=usb.description,frequency=lastFrequency,hopIndex=if(s.hopping)1 else 0,hopTotal=hopFrequencies.size,hopSwitches=0,message="Reception is active")
                    Hub.log("USB RX opened · ${usb.description} · ${s.sampleRate} S/s · KAT LO $center Hz")
                    bytes=0L;var intervalBytes=0L;var refresh=SystemClock.elapsedRealtime()
                    val pendingFrequency=java.util.concurrent.atomic.AtomicReference<Long?>(null)
                    val switching=java.util.concurrent.atomic.AtomicBoolean(false)
                    coroutineScope {
                        val hopper=if(s.hopping)launch(Dispatchers.Default){
                            val schedule=HopSchedule(hopFrequencies,s.hopDwellMs,SystemClock.elapsedRealtime())
                            runHopTimer(schedule,SystemClock::elapsedRealtime){hop->
                                ensureActive()
                                switching.set(true)
                                try{
                                    val nextFrequency=hop.frequency
                                    val nextCenter=Wire.liveCenter(nextFrequency,s.sampleRate)
                                    // The hardware timer does not wait for IQ or a detected signal.
                                    usb.tune(nextCenter)
                                    pendingFrequency.set(nextFrequency)
                                    Hub.status.update{it.copy(frequency=nextFrequency,hopIndex=hop.index+1,hopTotal=hop.frequencies.size,hopSwitches=hop.switches)}
                                }finally{switching.set(false)}
                            }
                        }else null
                        try{
                            usb.read(hopping=s.hopping)receive@{chunk,n,generation->
                                ensureActive()
                                if(switching.get()||!usb.isCurrentGeneration(generation))return@receive
                                pendingFrequency.getAndSet(null)?.let{target->
                                    // Only this RX thread owns the decoder; the timer never waits for it.
                                    engine!!.flush()
                                    lastFrequency=target
                                    val targetCenter=Wire.liveCenter(target,s.sampleRate)
                                    engine=ReceivePipeline(s.copy(frequency=target),(target-targetCenter).toDouble(),Hub::add)
                                }
                                if(switching.get()||!usb.isCurrentGeneration(generation))return@receive
                                bytes+=n;intervalBytes+=n
                                    synchronized(fileLock){recording?.let{out->
                                        if(recorded+n>recordLimit||recordFile!!.usableSpace<100L*1024*1024){closeRecording();Hub.notice.value="IQ recording stopped: quota or low storage"}
                                        else{out.write(chunk,0,n);recorded+=n}
                                    }}
                                    engine!!.process(chunk,n)
                                    val now=SystemClock.elapsedRealtime()
                                    if(now-refresh>=200){
                                        val rate=intervalBytes*1000/(now-refresh).coerceAtLeast(1)
                                        // Hop owns frequency/status fields, so an in-flight block cannot overwrite them.
                                        val power=engine!!.powerDb;val spectrum=Spectrum.fft(chunk,n)
                                        Hub.status.update{it.copy(bytes=bytes,bytesPerSecond=rate,
                                            powerDb=power,spectrum=spectrum,recording=recordFile?.name.orEmpty(),recordingBytes=recorded)}
                                        refresh=now;intervalBytes=0
                                    }
                            }
                        }finally{withContext(NonCancellable){hopper?.cancelAndJoin()}}
                    }
                }
            }catch(_:CancellationException){}
            catch(e:Exception){Hub.log("RX stopped: ${e.message}");Hub.notice.value=e.message?:"USB reception interrupted"}
            finally{
                runCatching{engine?.flush()}.onFailure{Hub.log("KAT DSP finalization: ${it.message}")};engine?.close();radio=null;closeRecording();Hub.status.value=Hub.status.value.copy(running=false,message="Reception stopped",bytesPerSecond=0,recording="")
                withContext(NonCancellable){Hub.flush()};stopSelf()
            }
        }
    }
    private fun toggleRecording(){
        try{synchronized(fileLock){
            if(recording!=null){closeRecording();return}
            check(Hub.status.value.running){"Start reception first"}
            val s=Hub.settings.value;check(!s.hopping){"Disable Hop before IQ recording; recordings require a fixed frequency"}
            val dir=File(getExternalFilesDir(null),"recordings").apply{mkdirs()}
            check(dir.usableSpace>100L*1024*1024){"Not enough storage"}
            check(dir.listFiles().orEmpty().sumOf{it.length()}<s.recordingQuotaMb*1024L*1024L){"IQ quota reached; delete or export old recordings"}
            recordLimit=s.recordingQuotaMb*1024L*1024L-dir.listFiles().orEmpty().sumOf{it.length()}
            val center=Wire.liveCenter(lastFrequency,s.sampleRate)
            val file=File(dir,"kat_${center}Hz_${s.sampleRate}sps_rx${lastFrequency}Hz_${System.currentTimeMillis()}.cs8")
            recorded=0;recordFile=file;recording=file.outputStream().buffered(256*1024)
            val metadata=StateFormat.gson.toJsonTree(s).asJsonObject.apply{addProperty("center_frequency",center);addProperty("target_frequency",lastFrequency);addProperty("input_offset_hz",lastFrequency-center);addProperty("iq_format","CS8")}
            File(dir,file.name+".json").writeText(StateFormat.gson.toJson(metadata))
            Hub.status.value=Hub.status.value.copy(recording=file.name);Hub.log("IQ recording: ${file.name}")
        }}catch(e:Exception){Hub.notice.value=e.message?:"Recording did not start"}
    }
    private fun closeRecording(){synchronized(fileLock){runCatching{recording?.close()};if(recordFile!=null)Hub.log("IQ recording complete · $recorded bytes");recording=null;recordFile=null;Hub.status.value=Hub.status.value.copy(recording="")}}
    override fun onDestroy(){radio?.stop();job?.cancel();scope.cancel();closeRecording();wake?.let{if(it.isHeld)it.release()};super.onDestroy()}
    companion object {
        fun action(context:Context,action:String,device:String=""){context.startForegroundService(Intent(context,RadioService::class.java).setAction(action).putExtra("device",device))}
    }
}
