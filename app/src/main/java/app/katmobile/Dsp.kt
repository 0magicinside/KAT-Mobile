package app.katmobile

import kotlin.math.*

object Spectrum {
    fun fft(bytes:ByteArray,length:Int=bytes.size,unsigned:Boolean=false):List<Float>{
        val size=1024;if(length<size*2)return emptyList()
        val re=DoubleArray(size);val im=DoubleArray(size)
        for(i in 0 until size){
            val w=0.5-0.5*cos(2*PI*i/(size-1))
            if(unsigned){
                re[i]=((bytes[i*2].toInt() and 0xFF)-127.5)/128.0*w
                im[i]=((bytes[i*2+1].toInt() and 0xFF)-127.5)/128.0*w
            }else{
                re[i]=bytes[i*2]/128.0*w
                im[i]=bytes[i*2+1]/128.0*w
            }
        }
        var j=0
        for(i in 1 until size){var bit=size shr 1;while(j and bit!=0){j=j xor bit;bit=bit shr 1};j=j xor bit;if(i<j){val r=re[i];re[i]=re[j];re[j]=r;val q=im[i];im[i]=im[j];im[j]=q}}
        var len=2
        while(len<=size){val angle=-2*PI/len;val wr0=cos(angle);val wi0=sin(angle)
            for(start in 0 until size step len){var wr=1.0;var wi=0.0
                for(k in 0 until len/2){val a=start+k;val b=a+len/2;val r=re[b]*wr-im[b]*wi;val q=re[b]*wi+im[b]*wr
                    re[b]=re[a]-r;im[b]=im[a]-q;re[a]+=r;im[a]+=q
                    val next=wr*wr0-wi*wi0;wi=wr*wi0+wi*wr0;wr=next
                }
            };len*=2
        }
        return (0 until 256).map{bin->(0..3).maxOf{k->val i=(bin*4+k+size/2)%size;(20*log10(max(hypot(re[i],im[i])/(size/2),1e-5))).toFloat()}.coerceIn(-100f,6f)}
    }
}
