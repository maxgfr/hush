package io.github.maxgfr.hush.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// The web app's accent, oklch(0.5 0.17 285) and its dark-mode lift.
private val Light = lightColorScheme(
    primary = Color(0xFF5B4BC4),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFE6E1FF),
    onPrimaryContainer = Color(0xFF1B0D63),
    secondary = Color(0xFF5E5A71),
    background = Color(0xFFFBFAFE),
    surface = Color(0xFFFBFAFE),
    error = Color(0xFFB3261E),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFB0A4F5),
    onPrimary = Color(0xFF2A1B7A),
    primaryContainer = Color(0xFF41349E),
    onPrimaryContainer = Color(0xFFE6E1FF),
    secondary = Color(0xFFC8C3DC),
    background = Color(0xFF131218),
    surface = Color(0xFF131218),
    error = Color(0xFFF2B8B5),
)

@Composable
fun HushTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) Dark else Light, content = content)
}
