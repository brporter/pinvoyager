import SwiftUI

struct LegalDocument: Decodable, Identifiable, Sendable {
  let id: String
  let title: String
  let revision: String
  let text: String
}

enum BundledLegal {
  static func documents(bundle: Bundle = .main) async throws -> [LegalDocument] {
    guard let url = bundle.url(forResource: "legal", withExtension: "json") else {
      throw CocoaError(.fileNoSuchFile)
    }
    return try await Task.detached {
      struct Archive: Decodable { let documents: [LegalDocument] }
      return try JSONDecoder().decode(Archive.self, from: Data(contentsOf: url)).documents
    }.value
  }
}

struct BundledLegalView: View {
  @Environment(AppModel.self) private var model
  let id: String
  @State private var document: LegalDocument?
  @State private var error: String?
  var body: some View {
    ScrollView {
      if let document {
        VStack(alignment: .leading, spacing: 16) {
          Text("Bundled revision: \(document.revision)").font(.caption)
          Text(document.text).textSelection(.enabled)
          Link("View latest website version", destination: model.webOrigin.url(path: [id]))
        }.padding()
      } else if let error {
        ErrorNotice(message: error)
      } else {
        ProgressView("Opening saved document")
      }
    }.navigationTitle(document?.title ?? "Legal")
      .task {
        do {
          guard let document = try await BundledLegal.documents().first(where: { $0.id == id })
          else {
            throw CocoaError(.fileNoSuchFile)
          }
          self.document = document
        } catch { self.error = error.localizedDescription }
      }
  }
}
