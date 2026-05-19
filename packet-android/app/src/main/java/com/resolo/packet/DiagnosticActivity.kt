package com.resolo.packet

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import com.resolo.phantom.PhantomTunnel
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.concurrent.thread

/**
 * Connection diagnostic screen.
 *
 * Workflow for the operator:
 *   1. Turn Psiphon ON, open this screen, tap "Run Diagnostic", tap "Copy".
 *      Save that text (report A — tunnelled).
 *   2. Turn Psiphon OFF, run again, copy (report B — direct).
 *   3. Turn Packet ON, run again, copy (report C — Packet local proxy).
 *   4. Send all reports. Raw egress shows process routing; local proxy
 *      egress shows whether Packet's DirectSock path is actually alive.
 *
 * The raw probe runs on outbound sockets. Packet excludes its own package
 * from Android VpnService to avoid tunnel loops, so Packet ON must be judged
 * by the local proxy section in the native report.
 */
class DiagnosticActivity : Activity() {

    private val defaultUri =
        "trojan://humanity@172.64.152.23:443?path=%2Fassignment&security=tls" +
            "&host=www.creationlong.org&type=ws&sni=www.creationlong.org#%40InfoTech_VK"

    private lateinit var uriInput: EditText
    private lateinit var output: TextView
    private lateinit var outputScroll: ScrollView
    private lateinit var runButton: Button

    private val stamp = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US)
    private var runStartMs = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val pad = (16 * resources.displayMetrics.density).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
        }

        val headerRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, 0, 0, pad / 4)
        }
        headerRow.addView(Button(this).apply {
            text = "← Back"
            setOnClickListener { finish() }
        })
        headerRow.addView(TextView(this).apply {
            text = "  Packet Connection Diagnostic"
            textSize = 18f
        })
        root.addView(headerRow)

        root.addView(TextView(this).apply {
            text = "Run with Psiphon ON, Psiphon OFF, and Packet ON. " +
                "For Packet, compare LOCAL PROXY EGRESS; raw app egress is excluded from our VPN."
            textSize = 12f
            setPadding(0, 0, 0, pad / 4)
        })

        uriInput = EditText(this).apply {
            setText(defaultUri)
            inputType = InputType.TYPE_CLASS_TEXT or
                InputType.TYPE_TEXT_FLAG_MULTI_LINE
            textSize = 11f
            maxLines = 3
            setHorizontallyScrolling(false)
        }
        root.addView(uriInput)

        // ── Output fills all remaining space ──────────────────────────
        output = TextView(this).apply {
            typeface = android.graphics.Typeface.MONOSPACE
            textSize = 10f
            text = "Idle. Tap \"Run Diagnostic\" below.\n"
            setTextIsSelectable(true)
        }
        outputScroll = ScrollView(this).apply {
            addView(output)
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f
            )
        }
        root.addView(outputScroll)

        // ── Run + Copy pinned at the BOTTOM ───────────────────────────
        val buttonRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, pad / 2, 0, 0)
        }
        runButton = Button(this).apply {
            text = "Run Diagnostic"
            setOnClickListener { runProbe() }
        }
        val copyButton = Button(this).apply {
            text = "Copy"
            setOnClickListener { copyOutput() }
        }
        buttonRow.addView(runButton, lpWeight())
        buttonRow.addView(copyButton, lpWeight())
        root.addView(buttonRow)

        setContentView(root)
    }

    private fun lpWeight() = LinearLayout.LayoutParams(
        0,
        ViewGroup.LayoutParams.WRAP_CONTENT,
        1f
    )

    private fun runProbe() {
        val uri = uriInput.text.toString().trim()
        if (uri.isEmpty()) {
            Toast.makeText(this, "Enter a trojan:// URI", Toast.LENGTH_SHORT).show()
            return
        }
        runButton.isEnabled = false
        runStartMs = System.currentTimeMillis()
        val startedAt = stamp.format(Date(runStartMs))
        output.text = "Run started at $startedAt\nRunning probe... (a few seconds)\n"
        thread {
            val report = try {
                PhantomTunnel.runDiagnostic(uri)
            } catch (t: Throwable) {
                "ERROR invoking native diagnostic: ${t.message}\n" +
                    t.stackTraceToString()
            }
            val elapsedMs = System.currentTimeMillis() - runStartMs
            val header = buildString {
                append("RUN STARTED : $startedAt\n")
                append("RUN FINISHED: ${stamp.format(Date())}\n")
                append("DURATION    : ${elapsedMs} ms\n")
                append("─────────────────────────────────────────────\n")
            }
            Handler(Looper.getMainLooper()).post {
                output.text = header + report
                runButton.isEnabled = true
                outputScroll.post { outputScroll.scrollTo(0, 0) }
            }
        }
    }

    private fun copyOutput() {
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("packet-diagnostic", output.text))
        Toast.makeText(this, "Report copied", Toast.LENGTH_SHORT).show()
    }
}
