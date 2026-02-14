package com.aifoundation.app.data.network

import com.aifoundation.app.data.api.FederationApi
import okhttp3.OkHttpClient
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Retrofit
import retrofit2.converter.gson.GsonConverterFactory
import java.util.concurrent.TimeUnit

/**
 * Network client singleton for Federation Server connections
 */
object NetworkClient {

    private const val DEFAULT_TIMEOUT = 30L

    // Default to localhost - user can change this in settings
    private var baseUrl = "http://10.0.2.2:31415/"  // 10.0.2.2 is localhost from Android emulator (31415 = hybrid-server)

    private val loggingInterceptor = HttpLoggingInterceptor().apply {
        level = HttpLoggingInterceptor.Level.BODY
    }

    private val okHttpClient = OkHttpClient.Builder()
        .addInterceptor(loggingInterceptor)
        .connectTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
        .readTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
        .writeTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
        .build()

    private var retrofit: Retrofit = buildRetrofit()

    private var _federationApi: FederationApi = retrofit.create(FederationApi::class.java)
    val federationApi: FederationApi get() = _federationApi

    private fun buildRetrofit(): Retrofit {
        return Retrofit.Builder()
            .baseUrl(baseUrl)
            .client(okHttpClient)
            .addConverterFactory(GsonConverterFactory.create())
            .build()
    }

    /**
     * Update the server URL (e.g., when connecting to a different host)
     */
    fun setServerUrl(url: String) {
        val normalizedUrl = if (url.endsWith("/")) url else "$url/"
        if (normalizedUrl != baseUrl) {
            baseUrl = normalizedUrl
            retrofit = buildRetrofit()
            _federationApi = retrofit.create(FederationApi::class.java)
        }
    }

    /**
     * Get current server URL
     */
    fun getServerUrl(): String = baseUrl
}
