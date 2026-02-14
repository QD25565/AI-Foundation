package com.aifoundation.app.data.local

import android.content.Context
import android.content.SharedPreferences
import androidx.core.content.edit

/**
 * Persistent storage for AI-Foundation Deep Net identity and settings.
 * Stores device registration, server URL, and user preferences.
 */
class DeepNetPreferences(context: Context) {

    companion object {
        private const val PREFS_NAME = "ai_foundation_deep_net"

        // Identity keys
        private const val KEY_DEVICE_ID = "device_id"
        private const val KEY_DEVICE_NAME = "device_name"
        private const val KEY_FINGERPRINT = "fingerprint"
        private const val KEY_REGISTERED_AT = "registered_at"

        // Server keys
        private const val KEY_SERVER_URL = "server_url"
        private const val KEY_LAST_CONNECTED_AT = "last_connected_at"

        // Pairing keys
        private const val KEY_H_ID = "h_id"
        private const val KEY_PAIRING_TOKEN = "pairing_token"
        private const val KEY_TEAMBOOK_SERVER_URL = "teambook_server_url"

        // Default values
        // NOTE: 10.0.2.2 is Android emulator's special IP that maps to host machine's localhost.
        // Real phones on the same WiFi network should use the computer's actual local IP
        // (e.g., 192.168.1.100:31415). Find it via 'ipconfig' (Windows) or 'ifconfig' (Mac/Linux).
        const val DEFAULT_SERVER_URL = "http://10.0.2.2:31415"
        const val DEFAULT_TEAMBOOK_URL = "http://10.0.2.2:8080"
    }

    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    // ===== Device Identity =====

    /**
     * The registered device ID from the federation server.
     * Null if not yet registered.
     */
    var deviceId: String?
        get() = prefs.getString(KEY_DEVICE_ID, null)
        set(value) = prefs.edit { putString(KEY_DEVICE_ID, value) }

    /**
     * The device name used during registration.
     */
    var deviceName: String?
        get() = prefs.getString(KEY_DEVICE_NAME, null)
        set(value) = prefs.edit { putString(KEY_DEVICE_NAME, value) }

    /**
     * The device fingerprint used for identity verification.
     */
    var fingerprint: String?
        get() = prefs.getString(KEY_FINGERPRINT, null)
        set(value) = prefs.edit { putString(KEY_FINGERPRINT, value) }

    /**
     * Timestamp of when this device was first registered (epoch millis).
     */
    var registeredAt: Long
        get() = prefs.getLong(KEY_REGISTERED_AT, 0L)
        set(value) = prefs.edit { putLong(KEY_REGISTERED_AT, value) }

    /**
     * Check if this device has been registered before.
     */
    val isRegistered: Boolean
        get() = !deviceId.isNullOrEmpty()

    // ===== Server Settings =====

    /**
     * The federation server URL.
     */
    var serverUrl: String
        get() = prefs.getString(KEY_SERVER_URL, DEFAULT_SERVER_URL) ?: DEFAULT_SERVER_URL
        set(value) = prefs.edit { putString(KEY_SERVER_URL, value) }

    /**
     * Timestamp of last successful connection (epoch millis).
     */
    var lastConnectedAt: Long
        get() = prefs.getLong(KEY_LAST_CONNECTED_AT, 0L)
        set(value) = prefs.edit { putLong(KEY_LAST_CONNECTED_AT, value) }

    // ===== Operations =====

    /**
     * Save registration data after successful registration.
     */
    fun saveRegistration(deviceId: String, deviceName: String, fingerprint: String) {
        prefs.edit {
            putString(KEY_DEVICE_ID, deviceId)
            putString(KEY_DEVICE_NAME, deviceName)
            putString(KEY_FINGERPRINT, fingerprint)
            putLong(KEY_REGISTERED_AT, System.currentTimeMillis())
            putLong(KEY_LAST_CONNECTED_AT, System.currentTimeMillis())
        }
    }

    /**
     * Update last connected timestamp.
     */
    fun updateLastConnected() {
        lastConnectedAt = System.currentTimeMillis()
    }

    /**
     * Clear all stored data (for logout/reset).
     */
    fun clear() {
        prefs.edit { clear() }
    }

    /**
     * Clear only identity data (keeps server URL).
     */
    fun clearIdentity() {
        prefs.edit {
            remove(KEY_DEVICE_ID)
            remove(KEY_DEVICE_NAME)
            remove(KEY_REGISTERED_AT)
        }
    }

    // ===== Pairing (Human Integration) =====

    /** Human ID from pairing (e.g., "human-yourname") */
    var hId: String?
        get() = prefs.getString(KEY_H_ID, null)
        set(value) = prefs.edit { putString(KEY_H_ID, value) }

    /** Auth token from pairing */
    var pairingToken: String?
        get() = prefs.getString(KEY_PAIRING_TOKEN, null)
        set(value) = prefs.edit { putString(KEY_PAIRING_TOKEN, value) }

    /** Teambook HTTP API server URL */
    var teambookServerUrl: String
        get() = prefs.getString(KEY_TEAMBOOK_SERVER_URL, DEFAULT_TEAMBOOK_URL) ?: DEFAULT_TEAMBOOK_URL
        set(value) = prefs.edit { putString(KEY_TEAMBOOK_SERVER_URL, value) }

    /** Whether pairing is complete */
    val isPaired: Boolean
        get() = !hId.isNullOrEmpty() && !pairingToken.isNullOrEmpty()

    /** Save pairing data after successful pairing */
    fun savePairing(hId: String, token: String) {
        prefs.edit {
            putString(KEY_H_ID, hId)
            putString(KEY_PAIRING_TOKEN, token)
            putLong(KEY_LAST_CONNECTED_AT, System.currentTimeMillis())
        }
    }

    /** Clear pairing data */
    fun clearPairing() {
        prefs.edit {
            remove(KEY_H_ID)
            remove(KEY_PAIRING_TOKEN)
        }
    }
}
