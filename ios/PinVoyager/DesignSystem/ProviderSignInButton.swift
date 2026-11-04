import SwiftUI

struct ProviderSignInButton: View {
  let provider: String
  let action: () -> Void

  private var title: String { "Sign in with \(provider.capitalized)" }

  private var titleFont: Font {
    switch provider {
    case "apple": .system(size: 19, weight: .medium)
    case "google": .custom("GoogleSans-Regular", size: 17).weight(.medium)
    default: .system(size: 17, weight: .semibold)
    }
  }

  private var titleColor: Color {
    switch provider {
    case "apple": .black
    case "google": Color(red: 31 / 255, green: 31 / 255, blue: 31 / 255)
    default: Color(red: 94 / 255, green: 94 / 255, blue: 94 / 255)
    }
  }

  @ViewBuilder
  private var logo: some View {
    if provider == "apple" {
      // Apple's supplied 31×44 artwork includes padding. Render the complete file
      // at button height, centering its glyph in the same 20-point logo column.
      Image("AppleSignIn").renderingMode(.original).resizable()
        .frame(width: 31, height: 44).frame(width: 20, height: 44)
        .accessibilityHidden(true)
    } else {
      Image(provider == "google" ? "GoogleSignIn" : "MicrosoftSignIn")
        .renderingMode(.original).resizable().scaledToFit()
        .frame(width: 20, height: 20).accessibilityHidden(true)
    }
  }

  var body: some View {
    Button(action: action) {
      HStack(spacing: 12) {
        logo
        Text(title).lineLimit(1).minimumScaleFactor(0.75)
          .font(titleFont).foregroundStyle(titleColor)
        Spacer(minLength: 0)
      }
      .padding(.horizontal, 16).frame(maxWidth: .infinity).frame(height: 44)
      .background(.white, in: RoundedRectangle(cornerRadius: 8))
      .overlay(
        RoundedRectangle(cornerRadius: 8)
          .strokeBorder(Color(red: 116 / 255, green: 119 / 255, blue: 117 / 255), lineWidth: 1)
      )
      .contentShape(RoundedRectangle(cornerRadius: 8))
    }.buttonStyle(.plain).accessibilityIdentifier("sign-in.\(provider)")
  }
}
