package com.aifoundation.app.data.network

import com.aifoundation.app.data.api.TeambookApi
import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Retrofit
import retrofit2.converter.gson.GsonConverterFactory
import java.util.concurrent.TimeUnit

/**
 * Network client for ai-foundation-mobile-api (default port 8081).
 *
 * Two OkHttpClient flavours:
 *  - Standard (30s timeout) — for all REST calls
 *  - SSE (no read timeout) — for GET /api/events streaming connection
 */
object TeambookClient {

    private const val DEFAULT_TIMEOUT = 30L

    // Default to emulator localhost on port 8081 (ai-foundation-mobile-api default)
    private var baseUrl = "http://10.0.2.2:8081/"

    // Bearer token from pairing — set after successful pairing, cleared on unpair
    private var authToken: String? = null

    private val loggingInterceptor = HttpLoggingInterceptor().apply {
        level = HttpLoggingInterceptor.Level.BODY
    }

    private val authInterceptor = Interceptor { chain ->
        val request = authToken?.let { token ->
            chain.request().newBuilder()
                .addHeader("Authorization", "Bearer $token")
                .build()
        } ?: chain.request()
        chain.proceed(request)
    }

    private var okHttpClient = buildOkHttpClient()
    private var retrofit: Retrofit = buildRetrofit()
    private var _api: TeambookApi = retrofit.create(TeambookApi::class.java)

    val api: TeambookApi get() = _api

    private fun buildOkHttpClient(): OkHttpClient =
        OkHttpClient.Builder()
            .addInterceptor(authInterceptor)
            .addInterceptor(loggingInterceptor)
            .connectTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .readTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .writeTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .build()

    private fun buildRetrofit(): Retrofit =
        Retrofit.Builder()
            .baseUrl(baseUrl)
            .client(okHttpClient)
            .addConverterFactory(GsonConverterFactory.create())
            .build()

    private fun rebuild() {
        okHttpClient = buildOkHttpClient()
        retrofit = buildRetrofit()
        _api = retrofit.create(TeambookApi::class.java)
    }

    /**
     * OkHttpClient for SSE connections. No read timeout — the connection
     * intentionally stays open until the app closes it.
     */
    fun sseOkHttpClient(): OkHttpClient =
        OkHttpClient.Builder()
            .addInterceptor(authInterceptor)
            .connectTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.SECONDS)   // no timeout — SSE is streaming
            .writeTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .build()

    fun setServerUrl(url: String) {
        val normalised = if (url.endsWith("/")) url else "$url/"
        if (normalised != baseUrl) {
            baseUrl = normalised
            rebuild()
        }
    }

    fun getServerUrl(): String = baseUrl

    fun setAuthToken(token: String?) {
        authToken = token
        rebuild()
    }

    fun getAuthToken(): String? = authToken

    fun isAuthenticated(): Boolean = authToken != null
}
