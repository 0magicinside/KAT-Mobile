package app.katmobile

import org.junit.Test
import org.junit.Assert.*

class ProtocolTest {
    private val pattern="101001011001011010010110"
    private fun princeton():List<Pulse> = pattern.flatMap{if(it=='1')listOf(Pulse(true,1170),Pulse(false,390))else listOf(Pulse(true,390),Pulse(false,1170))}+listOf(Pulse(true,390),Pulse(false,11700))
    private fun fixed(te:Int,bits:Int=12):List<Pulse> = listOf(Pulse(true,te))+pattern.take(bits).flatMap{if(it=='1')listOf(Pulse(false,te*2),Pulse(true,te))else listOf(Pulse(false,te),Pulse(true,te*2))}+Pulse(false,te*47)
    private fun weather(humidity:Int=45):List<Pulse>{val bits="00010111"+"1000"+215.toString(2).padStart(12,'0')+"1111"+humidity.toString(2).padStart(8,'0');return bits.flatMap{listOf(Pulse(true,500),Pulse(false,if(it=='1')2000 else 1000))}+listOf(Pulse(true,500),Pulse(false,4000))}
    @Test fun princetonRequiresRepeatedFrame(){val r=ProtocolRecognizer();assertNull(r.identify(princeton(),"AM"));val match=r.identify(princeton(),"AM")!!;assertEquals("Princeton / EV1527",match.name);assertEquals(24,match.bits);assertTrue(match.evidence.contains("no CRC"))}
    @Test fun cameAndNiceAreSeparatedByTiming(){assertEquals("CAME",ProtocolRecognizer().identify(fixed(320)+fixed(320),"AM")?.name);assertEquals("Nice FLO",ProtocolRecognizer().identify(fixed(700)+fixed(700),"AM")?.name)}
    @Test fun came24IsRecognized(){assertEquals(24,ProtocolRecognizer().identify(fixed(320,24)+fixed(320,24),"AM")?.bits)}
    @Test fun differentFramesDoNotCountAsRepeatedSignal(){val a=princeton();val b=a.toMutableList();b[0]=Pulse(true,390);b[1]=Pulse(false,1170);val r=ProtocolRecognizer();assertNull(r.identify(a,"AM"));assertNull(r.identify(b,"AM"))}
    @Test fun weatherNeedsThreeRepeatedFramesAndReportsMeasurements(){val r=ProtocolRecognizer();assertNull(r.identify(weather()+weather(),"AM"));val c=r.identify(weather(),"AM")!!;assertEquals("Nexus / TFA TH",c.name);assertTrue(c.evidence.contains("21.5 °C"));assertTrue(c.evidence.contains("45 %"))}
    @Test fun invalidWeatherHumidityIsNotNexus(){val r=ProtocolRecognizer().identify(weather(150)+weather(150)+weather(150),"AM");assertTrue(r==null||r.name!="Nexus / TFA TH")}
    @Test fun rubicsonChecksumIsCheckedInsteadOfCallingItNexus(){val r=ProtocolRecognizer().identify(weather(21)+weather(21),"AM");assertEquals("Rubicson / Solight TE44",r?.name);assertTrue(r!!.evidence.contains("CRC8 OK"))}
    @Test fun fskRawImportPreservesModulationAndDoesNotPretendToBeOok(){val raw=StateFormat.rawSub(Capture(pulses=princeton()+princeton())).replace("FuriHalSubGhzPresetOok650Async","FuriHalSubGhzPreset2FSKDev476Async");val c=StateFormat.importRawSub(raw);assertEquals("FM",c.modulation);assertEquals("Unknown",c.protocol)}
    @Test fun randomDurationsAndFmAreNeverRelabeledAsFixedCode(){val p=(0..80).map{Pulse(it%2==0,50+it*47)};assertNull(ProtocolRecognizer().identify(p,"AM"));assertNull(ProtocolRecognizer().identify(princeton()+princeton(),"FM"))}
    @Test fun rawImportRecognizesFamilyWithoutRewritingFobMetadata(){val c=Capture(pulses=princeton()+princeton());val imported=StateFormat.importRawSub(StateFormat.rawSub(c));assertEquals("Princeton / EV1527",imported.protocol)}
    @Test fun signedIqActuallyReachesProtocolRecognizer(){
        val captures=mutableListOf<Capture>();val engine=SignalEngine(RadioSettings(),captures::add)
        val out=java.io.ByteArrayOutputStream()
        (princeton()+princeton()+princeton()+Pulse(false,25000)).forEach{p->repeat(p.duration_us*2){out.write(if(p.level)100 else 0);out.write(0)}}
        out.toByteArray().toList().chunked(16384).forEach{engine.process(it.toByteArray())};engine.flush()
        assertTrue(captures.any{it.protocol=="Princeton / EV1527"})
    }
    @Test fun olderSessionsGainRecognitionFieldSafely(){val state=StateFormat.decode("""{"captures":[{"id":"old","frequency":433920000,"pulses":[]}]}""");assertEquals("",state.captures.single().recognition)}
}
