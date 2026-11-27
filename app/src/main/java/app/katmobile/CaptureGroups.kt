package app.katmobile

/** Presentation only: grouping never changes, merges or removes recorded waveforms. */
data class CaptureGroup(val protocol:String,val records:List<Capture>){
    val latest get()=records.maxOf{it.time}
    val modulation get()=records.map{it.modulation}.distinct().sorted().joinToString(" / ")
}
fun captureGroups(captures:List<Capture>)=captures.groupBy{it.protocol.trim().ifBlank{"Unknown"}}
    .map{(name,records)->CaptureGroup(name,records.sortedByDescending{it.time})}
    .sortedWith(compareByDescending<CaptureGroup>{it.latest}.thenBy{it.protocol})
fun recordCount(count:Int)="$count "+if(count==1)"capture"else"captures"
