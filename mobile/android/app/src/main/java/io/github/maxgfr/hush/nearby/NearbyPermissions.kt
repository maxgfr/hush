package io.github.maxgfr.hush.nearby

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat

/**
 * The runtime permissions each nearby medium needs. Scanning never derives a location
 * (`neverForLocation`), but Android 10 to 12 still require location to scan.
 */
object NearbyPermissions {
    fun bluetooth(): List<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            listOf(
                Manifest.permission.BLUETOOTH_SCAN,
                Manifest.permission.BLUETOOTH_ADVERTISE,
                Manifest.permission.BLUETOOTH_CONNECT,
            )
        } else {
            listOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    fun wifiDirect(): List<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            listOf(Manifest.permission.NEARBY_WIFI_DEVICES)
        } else {
            listOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    /** Everything "Nearby" asks for at once, plus the notification of the foreground service. */
    fun all(): List<String> {
        val extra = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            listOf(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            emptyList()
        }
        return (bluetooth() + wifiDirect() + extra).distinct()
    }

    fun has(context: Context, permissions: List<String>): Boolean = permissions.all {
        ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
    }

    /** What is still missing for Bluetooth and Wi-Fi Direct (the local network needs nothing). */
    fun missing(context: Context): List<String> = (bluetooth() + wifiDirect()).distinct().filterNot {
        ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
    }
}
