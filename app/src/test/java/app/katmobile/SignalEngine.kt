package app.katmobile

import kotlin.math.*

// Historical simplified baseline for comparison tests; never packaged in the app.
class SignalEngine(private val s:RadioSettings,private val capture:(Capture)->Unit) {
    private val mode=if(s.modulation=="FM")"FM"else"AM"
    private val step=(s.sampleRate/250_000).coerceAtLeast(1)
    private val dt=step*1_000_000.0/s.sampleRate
    private var sumI=0.0;private var sumQ=0.0;private var count=0
    private var previousI=0.0;private var previousQ=0.0;private var noise=0.0001
    private var current=false;private var run=0.0;private var fmCenter=0.0
    private var quiet=false
    private val pulses=ArrayList<Pulse>()
    private val recognizer=ProtocolRecognizer()
    var frequency=s.frequency
    var active=false;private set
    var powerDb=-100f;private set
    fun reset(){pulses.clear();run=0.0;current=false;active=false;count=0;sumI=0.0;sumQ=0.0;previousI=0.0;previousQ=0.0;noise=0.0001;fmCenter=0.0;quiet=false}
    fun process(bytes:ByteArray,length:Int=bytes.size){
        require(length in 0..bytes.size&&length%2==0)
        var total=0.0;var n=0
        var p=0
        while(p<length){
            sumI+=bytes[p].toDouble()/128;sumQ+=bytes[p+1].toDouble()/128;count++;p+=2
            if(count<step)continue
            val i=sumI/count;val q=sumQ/count;count=0;sumI=0.0;sumQ=0.0
            val power=i*i+q*q;total+=power;n++
            val threshold=noise*10.0.pow(s.thresholdDb/10.0)
            val carrier=power>max(threshold,0.00001)
            if(!carrier)noise=(noise*0.995+power*0.005).coerceAtLeast(0.0000001)
            val fm=atan2(previousI*q-previousQ*i,previousI*i+previousQ*q)
            previousI=i;previousQ=q
            val level=if(mode=="AM")carrier else {
                if(!carrier)fmCenter=0.0 else fmCenter=fmCenter*0.999+fm*0.001
                carrier&&fm>fmCenter
            }
            if(mode=="FM"&&!carrier){
                if(!quiet){if(run>=s.minimumPulseUs&&(pulses.isNotEmpty()||current))pulses.add(Pulse(current,run.roundToInt()));run=0.0;current=false;quiet=true}
                run+=dt;if(run>=s.burstGapUs)finish();active=false;continue
            }
            if(quiet){quiet=false;run=0.0;current=level}
            if(level==current)run+=dt else {
                if(run>=s.minimumPulseUs)pulses.add(Pulse(current,run.coerceAtMost(200000.0).roundToInt()))
                current=level;run=dt
            }
            active=carrier||pulses.isNotEmpty()
            if((!current&&run>=s.burstGapUs)||pulses.size>=8192)finish()
        }
        if(n>0)powerDb=(10*log10(max(total/n,1e-10))).toFloat().coerceIn(-100f,6f)
    }
    private fun finish(){
        if(pulses.size>=8){
            if(!current&&run>=s.minimumPulseUs&&pulses.lastOrNull()?.level==true)pulses.add(Pulse(false,run.coerceAtMost(200000.0).roundToInt()))
            val match=recognizer.identify(pulses,mode,frequency)
            if(match!=null||s.showUnknown)capture(Capture(frequency=frequency,modulation=mode,
                protocol=match?.name?:"Unknown",recognition=match?.evidence.orEmpty(),pulses=pulses.toList()))
        }
        pulses.clear();active=false
    }
    fun flush(){if(current&&run>=s.minimumPulseUs)pulses.add(Pulse(current,run.roundToInt()));finish();reset()}
}

