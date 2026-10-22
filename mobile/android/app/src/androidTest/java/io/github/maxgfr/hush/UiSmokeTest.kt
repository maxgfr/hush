package io.github.maxgfr.hush

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Launch, create a link: its QR code and its text appear. */
@RunWith(AndroidJUnit4::class)
class UiSmokeTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private fun text(id: Int) = rule.activity.getString(id)

    @Test
    fun creatingALinkShowsItsQrCodeAndText() {
        rule.onNodeWithText(text(R.string.new_conversation)).performClick()
        rule.onNodeWithText(text(R.string.create_link)).performScrollTo().performClick()
        rule.waitUntil(15_000) { rule.onAllNodes(hasTestTag("share-link")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag("qr").assertIsDisplayed()
        val link = rule.onNodeWithTag("share-link").fetchSemanticsNode().config[SemanticsProperties.Text].joinToString()
        assertTrue(link, link.startsWith("https://maxgfr.github.io/hush/#"))
        rule.onNodeWithTag("status").assertExists()
    }
}
