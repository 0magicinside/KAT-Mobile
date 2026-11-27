package app.katmobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class Vault(private val context:Context) {
    private val file=AtomicFile(File(context.filesDir,"session.enc"))
    private fun key():SecretKey {
        val store=KeyStore.getInstance("AndroidKeyStore").apply{load(null)}
        return store.getKey("kat-mobile-session",null) as? SecretKey ?: KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore").apply{
            init(KeyGenParameterSpec.Builder("kat-mobile-session",KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Synchronized fun save(state:Saved){
        val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply{init(Cipher.ENCRYPT_MODE,key())}
        val encrypted=cipher.iv+cipher.doFinal(StateFormat.gson.toJson(state).toByteArray())
        val out=file.startWrite();try{out.write(encrypted);file.finishWrite(out)}catch(e:Exception){file.failWrite(out);throw e}
    }
    @Synchronized fun load():Saved {
        if(!file.baseFile.exists()&&!File(file.baseFile.path+".bak").exists())return Saved()
        val bytes=file.openRead().use{it.readBytes()};require(bytes.size in 28..32*1024*1024)
        val cipher=Cipher.getInstance("AES/GCM/NoPadding").apply{init(Cipher.DECRYPT_MODE,key(),GCMParameterSpec(128,bytes.copyOfRange(0,12)))}
        return StateFormat.decode(String(cipher.doFinal(bytes.copyOfRange(12,bytes.size))))
    }
}
