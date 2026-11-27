package app.katmobile

import kotlinx.coroutines.ensureActive

/** Fixed dwell rotation; signal detection never pauses the schedule. */
internal class HopSchedule(val frequencies:List<Long>,private val dwellMs:Int,startedAt:Long){
    init{require(frequencies.isNotEmpty());require(dwellMs>0)}
    var index=0;private set
    var switches=0L;private set
    private var tunedAt=startedAt
    val frequency get()=frequencies[index]
    fun remainingMs(now:Long)=(dwellMs-(now-tunedAt)).coerceAtLeast(0L)
    fun advance(now:Long):Boolean{
        if(now-tunedAt<dwellMs)return false
        index=(index+1)%frequencies.size;switches++;tunedAt=now
        return true
    }
}

/** Runs without any IQ input or signal-detection callback. */
internal suspend fun runHopTimer(schedule:HopSchedule,clock:()->Long,wait:suspend(Long)->Unit={kotlinx.coroutines.delay(it)},onHop:(HopSchedule)->Unit){
    while(true){
        kotlinx.coroutines.currentCoroutineContext().ensureActive()
        wait(schedule.remainingMs(clock()).coerceAtLeast(1L))
        kotlinx.coroutines.currentCoroutineContext().ensureActive()
        if(schedule.advance(clock()))onHop(schedule)
    }
}
