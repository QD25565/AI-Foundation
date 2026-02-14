package com.aifoundation.app.data.network

import com.aifoundation.app.data.api.TeambookApi
import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Retrofit
import retrofit2.converter.gson.GsonConverterFactory
import java.util.concurrent.TimeUnit

/**
 * Network client for the AI-Foundation HTTP API (ai-foundation-http).
 * Adds Bearer token authentication via interceptor.
 */
object TeambookClient {

    private const val DEFAULT_TIMEOUT = 30L

    // Default to emulator localhost on port 8080 (ai-foundation-http default)
    private var baseUrl = "http://10.0.2.2:8080/"

    // Auth token from pairing - set after successful pairing
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

    private fun buildOkHttpClient(): OkHttpClient {
        return OkHttpClient.Builder()
            .addInterceptor(authInterceptor)
            .addInterceptor(loggingInterceptor)
            .connectTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .readTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .writeTimeout(DEFAULT_TIMEOUT, TimeUnit.SECONDS)
            .build()
    }

    private fun buildRetrofit(): Retrofit {
        return Retrofit.Builder()
            .baseUrl(baseUrl)
            .client(okHttpClient)
            .addConverterFactory(GsonConverterFactory.create())
            .build()
    }

    private fun rebuild() {
        okHttpClient = buildOkHttpClient()
        retrofit = buildRetrofit()
        _api = retrofit.create(TeambookApi::class.java)
    }

    /**
     * Set the server URL (e.g., "http://192.168.1.100:8080")
     */
    fun setServerUrl(url: String) {
        val normalizedUrl = if (url.endsWith("/")) url else "$url/"
        if (normalizedUrl != baseUrl) {
            baseUrl = normalizedUrl
            rebuild()
        }
    }

    fun getServerUrl(): String = baseUrl

    /**
     * Set the auth token (from pairing or saved preferences)
     */
    fun setAuthToken(token: String?) {
        authToken = token
        // Rebuild client to pick up the new token in interceptor closure
        rebuild()
    }

    fun getAuthToken(): String? = authToken

    fun isAuthenticated(): Boolean = authToken != null
}
