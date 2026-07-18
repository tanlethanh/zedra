package dev.zedra.app

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.net.Uri
import android.os.Build
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayOutputStream

/** Decodes, downscales, and re-encodes a picked/pasted image off the main thread. */
object ImageAcquire {
    private const val MAX_DIMENSION = 2048
    private const val JPEG_QUALITY = 80

    /** Power-of-two downsample factor so the decoded long edge is close to (but >=) MAX_DIMENSION. */
    private fun sampleSizeFor(longEdge: Int): Int =
        if (longEdge > MAX_DIMENSION) Integer.highestOneBit(longEdge / MAX_DIMENSION).coerceAtLeast(1) else 1

    fun processUri(context: Context, uri: Uri, callbackId: Int) {
        Thread {
            try {
                val original = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    val source = ImageDecoder.createSource(context.contentResolver, uri)
                    ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
                        val longEdge = maxOf(info.size.width, info.size.height)
                        decoder.setTargetSampleSize(sampleSizeFor(longEdge))
                    }
                } else {
                    decodeLegacyBitmap(context, uri)
                }

                val longEdge = maxOf(original.width, original.height)
                val scaled = if (longEdge > MAX_DIMENSION) {
                    val scale = MAX_DIMENSION.toFloat() / longEdge
                    Bitmap.createScaledBitmap(
                        original,
                        (original.width * scale).toInt().coerceAtLeast(1),
                        (original.height * scale).toInt().coerceAtLeast(1),
                        true,
                    )
                } else {
                    original
                }

                val output = ByteArrayOutputStream()
                scaled.compress(Bitmap.CompressFormat.JPEG, JPEG_QUALITY, output)

                MainActivity.nativeImageAcquireResult(callbackId, output.toByteArray(), "jpg")
            } catch (error: Throwable) {
                MainActivity.nativeImageAcquireError(callbackId, error.message ?: "image processing failed")
            }
        }.start()
    }

    private fun decodeLegacyBitmap(context: Context, uri: Uri): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        context.contentResolver.openInputStream(uri).use {
            BitmapFactory.decodeStream(it, null, bounds)
        }
        val longEdge = maxOf(bounds.outWidth, bounds.outHeight)
        val decodeOptions = BitmapFactory.Options().apply { inSampleSize = sampleSizeFor(longEdge) }
        val bitmap = context.contentResolver.openInputStream(uri)?.use {
            BitmapFactory.decodeStream(it, null, decodeOptions)
        } ?: throw IllegalStateException("could not decode bitmap")
        val exif = context.contentResolver.openInputStream(uri)?.use { ExifInterface(it) }
            ?: return bitmap
        val transform = Matrix().apply {
            if (exif.isFlipped) postScale(-1f, 1f)
            if (exif.rotationDegrees != 0) postRotate(exif.rotationDegrees.toFloat())
            val decodedLongEdge = maxOf(bitmap.width, bitmap.height)
            if (decodedLongEdge > MAX_DIMENSION) {
                val scale = MAX_DIMENSION.toFloat() / decodedLongEdge
                postScale(scale, scale)
            }
        }
        return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, transform, true)
    }
}
