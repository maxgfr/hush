package io.github.maxgfr.hush.nearby.tcp

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import java.net.InetAddress

/**
 * `_hush._tcp` over Android's NSD (mDNS on the current network). Instance names are the tokens'
 * hex, which change every 10 minutes and mean nothing to whoever hears them.
 */
class NsdDiscovery(context: Context) : ServiceDiscovery {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val registered = HashMap<String, NsdManager.RegistrationListener>()
    private var discovery: NsdManager.DiscoveryListener? = null
    private var multicast: WifiManager.MulticastLock? = null
    @Volatile private var wanted: Set<String> = emptySet()
    @Volatile private var found: (String, InetAddress, Int) -> Unit = { _, _, _ -> }
    // NSD resolves one service at a time before Android 14.
    private val resolveQueue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false

    @Synchronized
    override fun publish(services: Map<String, Int>) {
        for (name in registered.keys - services.keys) {
            registered.remove(name)?.let { runCatching { nsd.unregisterService(it) } }
        }
        for ((name, port) in services) {
            if (name in registered) continue
            val info = NsdServiceInfo().apply {
                serviceName = name
                serviceType = ServiceDiscovery.SERVICE_TYPE
                setPort(port)
            }
            val listener = object : NsdManager.RegistrationListener {
                override fun onServiceRegistered(info: NsdServiceInfo) {}
                override fun onRegistrationFailed(info: NsdServiceInfo, code: Int) {
                    synchronized(this@NsdDiscovery) { registered.remove(name) }
                }
                override fun onServiceUnregistered(info: NsdServiceInfo) {}
                override fun onUnregistrationFailed(info: NsdServiceInfo, code: Int) {}
            }
            registered[name] = listener
            runCatching { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener) }
                .onFailure { registered.remove(name) }
        }
    }

    @Synchronized
    override fun browse(names: Set<String>, found: (name: String, host: InetAddress, port: Int) -> Unit) {
        wanted = names
        this.found = found
        if (names.isEmpty()) {
            stopBrowsing()
            return
        }
        if (discovery != null) return
        multicast = wifi?.createMulticastLock("hush")?.apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, code: Int) {
                synchronized(this@NsdDiscovery) { if (discovery === this) discovery = null }
            }
            override fun onStopDiscoveryFailed(serviceType: String, code: Int) {}
            override fun onServiceLost(info: NsdServiceInfo) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                if (info.serviceName.lowercase() in wanted) enqueue(info)
            }
        }
        discovery = listener
        runCatching { nsd.discoverServices(ServiceDiscovery.SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener) }
            .onFailure { discovery = null }
    }

    @Synchronized
    override fun stop() {
        publish(emptyMap())
        wanted = emptySet()
        stopBrowsing()
    }

    private fun stopBrowsing() {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        multicast?.let { runCatching { it.release() } }
        multicast = null
        resolveQueue.clear()
    }

    @Synchronized
    private fun enqueue(info: NsdServiceInfo) {
        resolveQueue.addLast(info)
        if (!resolving) resolveNext()
    }

    @Synchronized
    private fun resolveNext() {
        val info = resolveQueue.removeFirstOrNull()
        if (info == null) {
            resolving = false
            return
        }
        resolving = true
        @Suppress("DEPRECATION") // registerServiceInfoCallback is Android 14+ only.
        val ok = runCatching {
            nsd.resolveService(info, object : NsdManager.ResolveListener {
                override fun onResolveFailed(info: NsdServiceInfo, code: Int) = resolveNext()
                override fun onServiceResolved(info: NsdServiceInfo) {
                    val host = hostOf(info)
                    val name = info.serviceName.lowercase()
                    if (host != null && name in wanted) found(name, host, info.port)
                    resolveNext()
                }
            })
        }.isSuccess
        if (!ok) resolveNext()
    }

    private fun hostOf(info: NsdServiceInfo): InetAddress? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            info.hostAddresses.firstOrNull()
        } else {
            @Suppress("DEPRECATION")
            info.host
        }
}
