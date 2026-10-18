package net.wardriver.mobile.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.googlefonts.Font
import androidx.compose.ui.text.googlefonts.GoogleFont
import androidx.compose.ui.unit.sp
import net.wardriver.mobile.R

private val fontProvider =
    androidx.compose.ui.text.googlefonts.GoogleFont.Provider(
        providerAuthority = "com.google.android.gms.fonts",
        providerPackage = "com.google.android.gms",
        certificates = R.array.com_google_android_gms_fonts_certs,
    )

private val Orbitron =
    FontFamily(
        Font(googleFont = GoogleFont("Orbitron"), fontProvider = fontProvider, weight = FontWeight.Bold),
        Font(googleFont = GoogleFont("Orbitron"), fontProvider = fontProvider, weight = FontWeight.SemiBold),
        Font(googleFont = GoogleFont("Orbitron"), fontProvider = fontProvider, weight = FontWeight.Medium),
    )

private val SpaceGrotesk =
    FontFamily(
        Font(googleFont = GoogleFont("Space Grotesk"), fontProvider = fontProvider, weight = FontWeight.Normal),
        Font(googleFont = GoogleFont("Space Grotesk"), fontProvider = fontProvider, weight = FontWeight.Medium),
        Font(googleFont = GoogleFont("Space Grotesk"), fontProvider = fontProvider, weight = FontWeight.SemiBold),
    )

val PackTypography =
    Typography(
        displayLarge = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Bold, fontSize = 32.sp, color = PackColor.Text),
        displayMedium = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Bold, fontSize = 28.sp, color = PackColor.Text),
        displaySmall = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.SemiBold, fontSize = 24.sp, color = PackColor.Text),
        headlineLarge = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.SemiBold, fontSize = 22.sp, color = PackColor.Text),
        headlineMedium = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.SemiBold, fontSize = 20.sp, color = PackColor.Text),
        headlineSmall = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 18.sp, color = PackColor.Text),
        titleLarge = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 16.sp, color = PackColor.Text),
        titleMedium = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 14.sp, color = PackColor.Text),
        titleSmall = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 12.sp, color = PackColor.Cyan, letterSpacing = 1.sp),
        bodyLarge = TextStyle(fontFamily = SpaceGrotesk, fontWeight = FontWeight.Normal, fontSize = 16.sp, color = PackColor.Text),
        bodyMedium = TextStyle(fontFamily = SpaceGrotesk, fontWeight = FontWeight.Normal, fontSize = 14.sp, color = PackColor.Text),
        bodySmall = TextStyle(fontFamily = SpaceGrotesk, fontWeight = FontWeight.Normal, fontSize = 12.sp, color = PackColor.Muted),
        labelLarge = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 12.sp, color = PackColor.Cyan, letterSpacing = 1.2.sp),
        labelMedium = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 11.sp, color = PackColor.Cyan, letterSpacing = 1.sp),
        labelSmall = TextStyle(fontFamily = Orbitron, fontWeight = FontWeight.Medium, fontSize = 10.sp, color = PackColor.Muted, letterSpacing = 0.8.sp),
    )
