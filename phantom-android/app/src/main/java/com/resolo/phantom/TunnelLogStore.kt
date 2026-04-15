package com.resolo.phantom

import android.content.Context
import java.io.File

object TunnelLogStore {
    private const val FILE_NAME = "phantom-tunnel.log"
    private const val MAX_LOG_ENTRIES = 200

    fun load(context: Context): List<String> {
        val file = logFile(context)
        if (!file.exists()) {
            return emptyList()
        }

        return file.readLines(Charsets.UTF_8).filter { it.isNotBlank() }
    }

    fun append(context: Context, message: String) {
        val trimmed = message.trim()
        if (trimmed.isEmpty()) {
            return
        }

        val lines = load(context).toMutableList()
        lines.add(trimmed)
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
}
