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
    private val lock = Any()
    private var cachedLines: MutableList<String>? = null

    fun load(context: Context): List<String> {
        synchronized(lock) {
            return loadCachedLines(context.applicationContext).toList()
        }
    }

    fun append(context: Context, message: String) {
        val normalizedLines = normalizeLines(message)
        if (normalizedLines.isEmpty()) {
            return
        }

        val appContext = context.applicationContext
        synchronized(lock) {
            val lines = loadCachedLines(appContext)
            val previousSize = lines.size
            lines.addAll(normalizedLines)
            if (lines.size > MAX_LOG_ENTRIES) {
                lines.subList(0, lines.size - MAX_LOG_ENTRIES).clear()
            }

            if (previousSize == 0 || lines.size < previousSize + normalizedLines.size) {
                writeLines(appContext, lines)
            } else {
                appendLines(appContext, normalizedLines)
            }
        }

        TunnelEvents.broadcastLog(appContext)
    }

    fun clear(context: Context) {
        val appContext = context.applicationContext
        synchronized(lock) {
            cachedLines = mutableListOf()
            writeLines(appContext, emptyList())
        }
        TunnelEvents.broadcastLog(appContext)
    }

    private fun writeLines(context: Context, lines: List<String>) {
        val file = logFile(context)
        file.parentFile?.mkdirs()
        if (lines.isEmpty()) {
            if (file.exists()) {
                file.delete()
            }
            return
        }

        file.bufferedWriter(Charsets.UTF_8).use { writer ->
            lines.forEachIndexed { index, line ->
                if (index > 0) {
                    writer.newLine()
                }
                writer.write(line)
            }
        }
    }

    private fun appendLines(context: Context, lines: List<String>) {
        if (lines.isEmpty()) {
            return
        }

        val file = logFile(context)
        file.parentFile?.mkdirs()
        val prefix = if (file.exists() && file.length() > 0) "\n" else ""
        file.appendText(prefix + lines.joinToString(separator = "\n"), Charsets.UTF_8)
    }

    private fun logFile(context: Context): File {
        return File(context.filesDir, FILE_NAME)
    }

    private fun loadCachedLines(context: Context): MutableList<String> {
        cachedLines?.let { return it }

        val loaded = if (logFile(context).exists()) {
            logFile(context).readLines(Charsets.UTF_8).filter { it.isNotBlank() }.toMutableList()
        } else {
            mutableListOf()
        }

        cachedLines = loaded
        return loaded
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
