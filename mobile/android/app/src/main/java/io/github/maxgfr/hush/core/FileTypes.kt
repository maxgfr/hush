package io.github.maxgfr.hush.core

/**
 * The type a received file is handed to other apps with. The type the sender declared is never
 * used: an "invoice.pdf" declared text/html would open in a browser. It comes from the saved
 * file's own extension, and only for kinds a viewer shows without running anything; the rest
 * (web pages, SVG, XML, scripts, packages, anything unknown) is opaque bytes.
 */
object FileTypes {
    const val OPAQUE = "application/octet-stream"

    /** [lookup]: the platform's extension → type table (`MimeTypeMap`). */
    fun forName(name: String, lookup: (String) -> String?): String {
        val ext = name.substringAfterLast('.', "").lowercase()
        if (ext.isEmpty() || ext.length > 10 || !ext.all { it in 'a'..'z' || it in '0'..'9' }) return OPAQUE
        val type = lookup(ext)?.lowercase() ?: return OPAQUE
        return if (viewable(type)) type else OPAQUE
    }

    private fun viewable(type: String): Boolean = when {
        // An image that is a document: it can carry script.
        type == "image/svg+xml" -> false
        type.startsWith("image/") || type.startsWith("audio/") || type.startsWith("video/") -> true
        else -> type in DOCUMENTS
    }

    private val DOCUMENTS = setOf(
        "text/plain",
        "text/csv",
        "application/pdf",
        "application/msword",
        "application/vnd.ms-excel",
        "application/vnd.ms-powerpoint",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/vnd.oasis.opendocument.text",
        "application/vnd.oasis.opendocument.spreadsheet",
        "application/vnd.oasis.opendocument.presentation",
    )
}
