package io.github.maxgfr.hush

import io.github.maxgfr.hush.core.FileTypes
import org.junit.Assert.assertEquals
import org.junit.Test

class FileTypesTest {
    // What MimeTypeMap answers for these extensions.
    private val platform = mapOf(
        "jpg" to "image/jpeg",
        "png" to "image/png",
        "mp4" to "video/mp4",
        "pdf" to "application/pdf",
        "txt" to "text/plain",
        "docx" to "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "html" to "text/html",
        "htm" to "text/html",
        "xhtml" to "application/xhtml+xml",
        "svg" to "image/svg+xml",
        "xml" to "text/xml",
        "js" to "text/javascript",
        "apk" to "application/vnd.android.package-archive",
    )

    private fun type(name: String) = FileTypes.forName(name, platform::get)

    @Test
    fun viewableKindsKeepTheirType() {
        assertEquals("image/jpeg", type("photo.JPG"))
        assertEquals("image/png", type("a.b.png"))
        assertEquals("video/mp4", type("clip.mp4"))
        assertEquals("application/pdf", type("invoice.pdf"))
        assertEquals("text/plain", type("notes.txt"))
        assertEquals(platform["docx"], type("report.docx"))
    }

    @Test
    fun whatCanRunScriptOrInstallIsOpaque() {
        for (name in listOf("page.html", "page.htm", "page.xhtml", "logo.svg", "data.xml", "app.js", "app.apk")) {
            assertEquals(name, FileTypes.OPAQUE, type(name))
        }
        assertEquals(FileTypes.OPAQUE, type("invoice.pdf.html"))
    }

    @Test
    fun unknownOrOddExtensionsAreOpaque() {
        assertEquals(FileTypes.OPAQUE, type("file"))
        assertEquals(FileTypes.OPAQUE, type("file."))
        assertEquals(FileTypes.OPAQUE, type(".hidden"))
        assertEquals(FileTypes.OPAQUE, type("archive.xyz"))
        assertEquals(FileTypes.OPAQUE, type("a.p df"))
        assertEquals(FileTypes.OPAQUE, type("a.verylongextension"))
    }
}
