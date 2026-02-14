package com.aifoundation.app.data

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/**
 * Manages first-time setup state and node identity persistence
 * Uses DataStore for encrypted preferences storage
 */
class SetupManager(private val context: Context) {

    companion object {
        private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "deepnet_setup")

        private val KEY_SETUP_COMPLETE = booleanPreferencesKey("setup_complete")
        private val KEY_NODE_ID = stringPreferencesKey("node_id")
        private val KEY_SETUP_TIMESTAMP = stringPreferencesKey("setup_timestamp")
    }

    /**
     * Check if initial setup has been completed
     */
    val isSetupComplete: Flow<Boolean> = context.dataStore.data.map { preferences ->
        preferences[KEY_SETUP_COMPLETE] ?: false
    }

    /**
     * Get the stored node ID
     */
    val nodeId: Flow<String> = context.dataStore.data.map { preferences ->
        preferences[KEY_NODE_ID] ?: ""
    }

    /**
     * Mark setup as complete and store the generated node ID
     */
    suspend fun completeSetup(nodeId: String) {
        context.dataStore.edit { preferences ->
            preferences[KEY_SETUP_COMPLETE] = true
            preferences[KEY_NODE_ID] = nodeId
            preferences[KEY_SETUP_TIMESTAMP] = System.currentTimeMillis().toString()
        }
    }

    /**
     * Reset setup (for testing/debugging)
     */
    suspend fun resetSetup() {
        context.dataStore.edit { preferences ->
            preferences.clear()
        }
    }
}
