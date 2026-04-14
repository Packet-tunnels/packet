package com.resolo.phantom

import android.annotation.SuppressLint
import android.app.Activity
import android.os.Bundle
import android.view.View
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView

class MainActivity : Activity() {
    private lateinit var statusText: TextView
    private lateinit var serverUrlInput: EditText
    private lateinit var secretInput: EditText
    private lateinit var listenPortInput: EditText
    private lateinit var cdnEdgeInput: EditText
    private lateinit var hostOverrideInput: EditText
    private lateinit var transportSpinner: Spinner
    private lateinit var logsView: TextView
    private lateinit var rootScrollView: ScrollView

    private var tunnelStartRequested = false

    private val logCallback = object : PhantomTunnel.LogCallback {
        override fun onLog(message: String) {
            runOnUiThread {
                appendLog(message.trimEnd())
            }
        }
    }

    @SuppressLint("SetTextI18n")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusText = findViewById(R.id.statusText)
        serverUrlInput = findViewById(R.id.serverUrlInput)
        secretInput = findViewById(R.id.secretInput)
        listenPortInput = findViewById(R.id.listenPortInput)
        cdnEdgeInput = findViewById(R.id.cdnEdgeInput)
        hostOverrideInput = findViewById(R.id.hostOverrideInput)
        transportSpinner = findViewById(R.id.transportSpinner)
        logsView = findViewById(R.id.logsView)
        rootScrollView = findViewById(R.id.rootScrollView)

        serverUrlInput.setText("http://piano-lessons.site")
        secretInput.setText("change-me")
        listenPortInput.setText("1080")

        transportSpinner.adapter = ArrayAdapter(
            this,
            android.R.layout.simple_spinner_item,
            listOf("Auto", "WebSocket", "HTTP")
        ).apply {
            setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }

        PhantomTunnel.setLogCallback(logCallback)

        appendLog("[APP] Android test app ready")
        appendLog("[APP] Rust JNI bridge loaded")
        appendLog("[APP] This target starts the Rust SOCKS5 client only")

        findViewById<Button>(R.id.startButton).setOnClickListener {
            startTunnel()
        }

        findViewById<Button>(R.id.testOutputButton).setOnClickListener {
            PhantomTunnel.emitTestOutput()
            statusText.text = "Rust test output emitted"
            appendLog("[APP] Requested Rust test output")
        }

        findViewById<Button>(R.id.clearLogsButton).setOnClickListener {
            logsView.text = ""
            statusText.text = "Logs cleared"
            appendLog("[APP] Logs cleared")
        }
    }

    private fun startTunnel() {
        if (tunnelStartRequested) {
            statusText.text = "Tunnel already started"
            appendLog("[APP] Start ignored because the Rust client has no stop API yet")
            return
        }

        val serverUrl = serverUrlInput.text.toString().trim()
        val secret = secretInput.text.toString().trim()
        val portText = listenPortInput.text.toString().trim()
        val cdnEdge = cdnEdgeInput.text.toString().trim()
        val hostOverride = hostOverrideInput.text.toString().trim()

        if (serverUrl.isEmpty()) {
            statusText.text = "Server URL is required"
            appendLog("[APP] Server URL is required")
            return
        }

        if (secret.isEmpty()) {
            statusText.text = "Secret is required"
            appendLog("[APP] Secret is required")
            return
        }

        val listenPort = portText.toIntOrNull()
        if (listenPort == null || listenPort !in 1..65535) {
            statusText.text = "Listen port must be between 1 and 65535"
            appendLog("[APP] Listen port must be between 1 and 65535")
            return
        }

        statusText.text = "Starting tunnel"
        appendLog("[APP] Starting tunnel on 127.0.0.1:$listenPort")

        if (cdnEdge.isNotEmpty() || hostOverride.isNotEmpty()) {
            PhantomTunnel.startClientCdn(
                serverUrl,
                secret,
                listenPort,
                cdnEdge,
                hostOverride,
                transportSpinner.selectedItemPosition
            )
            appendLog("[APP] Tunnel start requested in CDN mode")
        } else {
            PhantomTunnel.startClient(serverUrl, secret, listenPort)
            appendLog("[APP] Tunnel start requested")
        }

        tunnelStartRequested = true
        statusText.text = "Tunnel start requested"
    }

    private fun appendLog(message: String) {
        if (message.isBlank()) {
            return
        }

        val current = logsView.text?.toString().orEmpty()
        logsView.text = if (current.isBlank()) {
            message
        } else {
            "$current\n$message"
        }

        rootScrollView.post {
            rootScrollView.fullScroll(View.FOCUS_DOWN)
        }
    }
}
