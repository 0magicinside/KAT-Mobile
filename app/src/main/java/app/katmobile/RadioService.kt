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
    private var txJob:Job?=null
    /** Receiving stopped for a transmission and starts again after it: the receiver's end does not stop the service. */
    @Volatile private var pausedForTx=false
    @Volatile private var radio:SdrUsb?=null
    @Volatile private var activeDeviceName=""
    @Volatile private var activeIqFormat=IqFormat.CS8
    @Volatile private var activeSampleRate=2_000_000
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
        val action=intent?.action
        if(action=="START"||action=="TX"){
            val manager=getSystemService(UsbManager::class.java)
            val device=manager.deviceList.values.firstOrNull{
                it.deviceName==intent.getStringExtra("device")&&if(action=="TX")HackRfUsb.supported(it)else SdrDevices.supported(it)
            }
            if(device==null||!manager.hasPermission(device)){Hub.notice.value="The USB device is no longer available";if(action=="TX")Hub.pendingTx.set(null);stopIfIdle();return START_NOT_STICKY}
            try{startForeground(8,notification())}catch(e:Exception){Hub.notice.value=e.message?:"Reception did not start";if(action=="TX")Hub.pendingTx.set(null);stopIfIdle();return START_NOT_STICKY}
        }else if(!busy()){stopSelf();return START_NOT_STICKY}
        when(action){
            "START"->if(!busy())startReceiver(intent.getStringExtra("device").orEmpty())
            "TX"->transmit(intent.getStringExtra("device").orEmpty())
            "RECORD"->toggleRecording()
            else->{radio?.stop();job?.cancel();txJob?.cancel();closeRecording();stopSelf()}
        }
        return START_NOT_STICKY
    }
    private fun busy()=job?.isActive==true||txJob?.isActive==true
    private fun stopIfIdle(){if(!busy())stopSelf()}
    /** One transmission (Hub.pendingTx). Receiving pauses for it and resumes, as desktop KAT does with one HackRF. */
    private fun transmit(name:String){
        val request=Hub.pendingTx.getAndSet(null)?:run{stopIfIdle();return}
        if(txJob?.isActive==true){Hub.notice.value="A transmission is already on air";return}
        if(recording!=null){Hub.notice.value="Stop the IQ recording before transmitting";stopIfIdle();return}
        val receiving=job?.isActive==true
        val resumeDevice=activeDeviceName.ifBlank{name}
        txJob=scope.launch{
            Hub.status.update{it.copy(transmitting=true,message="Transmitting ${request.what}…")}
            Hub.txMessage.value="Transmitting ${request.what}…"
            var resume=receiving
            try{
                // The samples first: nothing is keyed until they are ready.
                val iq=NativeTx.iq(request.plan)
                if(receiving){pausedForTx=true;radio?.stop();job?.cancelAndJoin()}
                val manager=getSystemService(UsbManager::class.java)
                val device=manager.deviceList.values.firstOrNull{it.deviceName==name&&HackRfUsb.supported(it)}?:error("HackRF is unavailable")
                val s=Hub.settings.value
                HackRfUsb(manager,device).use{usb->usb.transmit(iq,request.carrier,s.txVga,s.txAmp){!isActive}}
                Hub.transmitted(request)
            }catch(e:CancellationException){resume=false;throw e}
            catch(e:Exception){
                Hub.log("TX failed: ${e.message}")
                Hub.txMessage.value="Transmission failed: ${e.message}"
                Hub.notice.value="Transmission failed: ${e.message}"
            }finally{
                pausedForTx=false
                Hub.status.update{it.copy(transmitting=false,message=Hub.txMessage.value)}
                if(resume&&scope.isActive)startReceiver(resumeDevice) else if(job?.isActive!=true)stopSelf()
            }
        }
    }
    private fun notification():Notification {
        val open=PendingIntent.getActivity(this,0,Intent(this,MainActivity::class.java),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop=PendingIntent.getService(this,1,Intent(this,RadioService::class.java).setAction("STOP"),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val label=Hub.status.value.radioKind?.label?:"USB"
        return Notification.Builder(this,"radio").setSmallIcon(R.drawable.ic_radio).setContentTitle("KAT Mobile · RX ($label)")
            .setContentText("${"%.3f".format(Hub.status.value.frequency/1e6)} MHz · $label reception")
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null,"Stop",stop).build()).build()
    }
    private fun startReceiver(name:String){
        val manager=getSystemService(UsbManager::class.java)
        val device=manager.deviceList.values.firstOrNull{it.deviceName==name&&SdrDevices.supported(it)}
        val kind=device?.let(SdrDevices::kindOf)
        if(device==null||kind==null||!manager.hasPermission(device)){Hub.notice.value="USB SDR device is unavailable or permission is missing";stopSelf();return}
        val s=Hub.settings.value
        val rate=if(kind==RadioKind.RTLSDR)s.rtlSampleRate else s.sampleRate
        activeDeviceName=device.deviceName
        activeSampleRate=rate
        activeIqFormat=if(kind==RadioKind.RTLSDR)IqFormat.CU8 else IqFormat.CS8
        Hub.status.value=RadioStatus(running=true,device=SdrDevices.displayName(device),radioKind=kind,activeSampleRate=rate,frequency=s.frequency,message="Opening ${kind.label} USB reception…")
        wake=getSystemService(PowerManager::class.java).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,"KAT:rx").apply{acquire(24*60*60*1000L)}
        job=scope.launch {
            var engine:ReceivePipeline?=null
            try{
                SdrDevices.open(manager,device).use{usb->
                    radio=usb
                    val usbRate=usb.sampleRate(s)
                    val iqFormat=usb.iqFormat
                    activeSampleRate=usbRate
                    activeIqFormat=iqFormat
                    val hopFrequencies=if(s.hopping)s.frequencies()else emptyList()
                    lastFrequency=hopFrequencies.firstOrNull()?:s.frequency
                    val center=Wire.liveCenter(lastFrequency,usbRate);usb.configure(s,center)
                    engine=ReceivePipeline(s.copy(sampleRate=usbRate,frequency=lastFrequency),(lastFrequency-center).toDouble(),iqFormat,usb.kind.label,Hub::add)
                    Hub.status.value=Hub.status.value.copy(radioKind=usb.kind,activeSampleRate=usbRate,firmware=usb.description,frequency=lastFrequency,hopIndex=if(s.hopping)1 else 0,hopTotal=hopFrequencies.size,hopSwitches=0,message="Reception is active")
                    Hub.log("${usb.kind.label} USB RX opened · ${usb.description} · $usbRate S/s · ${iqFormat.label} · KAT LO $center Hz")
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
                                    val nextCenter=Wire.liveCenter(nextFrequency,usbRate)
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
                                    val targetCenter=Wire.liveCenter(target,usbRate)
                                    engine=ReceivePipeline(s.copy(sampleRate=usbRate,frequency=target),(target-targetCenter).toDouble(),iqFormat,usb.kind.label,Hub::add)
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
                                        val rateBps=intervalBytes*1000/(now-refresh).coerceAtLeast(1)
                                        // Hop owns frequency/status fields, so an in-flight block cannot overwrite them.
                                        val power=engine!!.powerDb;val spectrum=Spectrum.fft(chunk,n,iqFormat.unsigned)
                                        Hub.status.update{it.copy(bytes=bytes,bytesPerSecond=rateBps,
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
                withContext(NonCancellable){Hub.flush()};if(!pausedForTx)stopSelf()
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
            val iq=activeIqFormat
            val rate=activeSampleRate
            val center=Wire.liveCenter(lastFrequency,rate)
            val file=File(dir,"kat_${center}Hz_${rate}sps_rx${lastFrequency}Hz_${System.currentTimeMillis()}.${iq.extension}")
            recorded=0;recordFile=file;recording=file.outputStream().buffered(256*1024)
            val metadata=StateFormat.gson.toJsonTree(s).asJsonObject.apply{addProperty("center_frequency",center);addProperty("target_frequency",lastFrequency);addProperty("sample_rate",rate);addProperty("input_offset_hz",lastFrequency-center);addProperty("iq_format",iq.label)}
            File(dir,file.name+".json").writeText(StateFormat.gson.toJson(metadata))
            Hub.status.value=Hub.status.value.copy(recording=file.name);Hub.log("IQ recording (${iq.label}): ${file.name}")
        }}catch(e:Exception){Hub.notice.value=e.message?:"Recording did not start"}
    }
    private fun closeRecording(){synchronized(fileLock){runCatching{recording?.close()};if(recordFile!=null)Hub.log("IQ recording complete · $recorded bytes");recording=null;recordFile=null;Hub.status.value=Hub.status.value.copy(recording="")}}
    override fun onDestroy(){radio?.stop();job?.cancel();scope.cancel();closeRecording();wake?.let{if(it.isHeld)it.release()};super.onDestroy()}
    companion object {
        fun action(context:Context,action:String,device:String=""){context.startForegroundService(Intent(context,RadioService::class.java).setAction(action).putExtra("device",device))}
    }
}
