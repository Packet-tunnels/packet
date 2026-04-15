package com.resolo.packet

import android.content.Context
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

object TunnelLogStore {
    private const val FILE_NAME = "packet.log"
    private const val MAX_LOG_ENTRIES = 1000
    private val timestampPattern = Regex("""^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}""")

    fun load(context: Context): List<String> {
        val file = logFile(context)
        if (!file.exists()) {
            return emptyList()
        }

        return file.readLines(Charsets.UTF_8).filter { it.isNotBlank() }
    }

    fun append(context: Context, message: String) {
        val normalizedLines = normalizeLines(message)
        if (normalizedLines.isEmpty()) {
            return
        }

        val lines = load(context).toMutableList()
        lines.addAll(normalizedLines)
        if (lines.size > MAX_LOG_ENTRIES) {
            lines.subList(0, lines.size - MAX_LOG_ENTRIES).clear()
        }

        writeLines(context, lines)
        TunnelEvents.broadcastLog(context)
    }

    fun clear(context: Context) {
        writeLines(context, emptyList())
        TunnelEvents.broadcastLog(context)
    }

    private fun writeLines(context: Context, lines: List<String>) {
        logFile(context).bufferedWriter(Charsets.UTF_8).use { writer ->
            lines.forEachIndexed { index, line ->
                if (index > 0) {
                    writer.newLine()
                }
                writer.write(line)
            }
        }
    }

    private fun logFile(context: Context): File {
        return File(context.filesDir, FILE_NAME)
    }

    private fun normalizeLines(message: String): List<String> {
        return message.lineSequence()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .map { line ->
                if (timestampPattern.containsMatchIn(line)) {
                    line
                } else {
                    "${timestampUtc()} $line"
                }
            }
            .toList()
    }

    private fun timestampUtc(): String {
        return SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US).apply {
            timeZone = TimeZone.getTimeZone("UTC")
        }.format(Date())
    }
}
