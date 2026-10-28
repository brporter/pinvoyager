import PinVoyagerData
import PinVoyagerDomain
import SwiftUI

struct MyVoyagesView: View {
  @Environment(AppModel.self) private var model
  @Environment(SceneRouter.self) private var router
  @State private var search = ""
  @State private var filter = "All"
  @State private var deleting: LocalVoyage?
  var body: some View {
    Group {
      if model.workspace == nil {
        ContentUnavailableView {
          Label("Sign in to see your voyages", systemImage: "suitcase")
        } description: {
          Text("Your drafts and photographs stay with your account.")
        } actions: {
          Button("Sign in") { router.showingSignIn = true }.buttonStyle(.borderedProminent)
        }
      } else {
        List {
          Picker("Visibility", selection: $filter) {
            ForEach(["All", "Drafts", "Public"], id: \.self) { Text($0) }
          }.pickerStyle(.segmented)
          if model.workspace?.scratch != nil {
            Button("Continue unfinished draft") { router.showingCreation = true }
          }
          if voyages.isEmpty {
            ContentUnavailableView {
              Label(
                search.isEmpty ? "No saved voyages" : "No matching saved voyages",
                systemImage: "suitcase")
            } actions: {
              Button("Create a voyage") { router.showingCreation = true }
            }
          }
          if let workspace = model.workspace {
            let deletions = workspace.voyages.filter { workspace.canUndoVoyageDeletion($0.id) }
            if !deletions.isEmpty {
              Section("Voyage deletion waiting to sync") {
                ForEach(deletions) { voyage in
                  VStack(alignment: .leading) {
                    Text(voyage.fields.title)
                    Button("Undo deletion") {
                      Task {
                        await model.perform {
                          try await model.repository.undoVoyageDeletion(
                            accountId: workspace.account.id, voyageId: voyage.id)
                        }
                      }
                    }.accessibilityIdentifier("voyage.undo.\(voyage.id)")
                  }
                }
              }
            }
          }
          ForEach(voyages) { voyage in
            NavigationLink(value: SceneDestination.localVoyage(voyage.id)) {
              VStack(alignment: .leading, spacing: 8) {
                Text(voyage.fields.title).font(.headline)
                HStack {
                  VisibilityLabel(published: voyage.confirmedPublished)
                  Text(status(voyage)).foregroundStyle(.secondary)
                }.font(.caption)
                Text(voyage.updatedAt, style: .date).font(.caption).foregroundStyle(.secondary)
              }.padding(.vertical, 8)
            }.swipeActions { Button("Delete", role: .destructive) { deleting = voyage } }
          }
        }
        .searchable(text: $search, prompt: "Search saved voyages")
        .refreshable { await model.refreshOwnerList() }
      }
    }
    .navigationTitle("My Voyages")
    .toolbar {
      Button("Create a voyage", systemImage: "plus") {
        router.requestCreation(hasWorkspace: model.workspace != nil)
      }
    }
    .confirmationDialog(
      "Delete \(deleting?.fields.title ?? "voyage") and its photos?",
      isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
      titleVisibility: .visible
    ) {
      Button(
        deletingIsLocalOnly ? "Delete from this iPhone" : "Delete voyage and its photos",
        role: .destructive
      ) {
        if let id = deleting?.id, let account = model.activeAccountId {
          let localOnly = deletingIsLocalOnly
          Task {
            await model.perform {
              if localOnly {
                guard let discard = model.localDiscard else { throw FileVaultError.missingSource }
                try await discard.deleteLocalOnly(accountId: account, voyageId: id)
              } else {
                try await model.repository.deleteVoyage(accountId: account, voyageId: id)
              }
            }
          }
        }
        deleting = nil
      }
    } message: {
      if deletingIsLocalOnly {
        Text(
          "This removes \(model.workspace?.photos(for: deleting?.id ?? UUID()).filter { $0.serverId == nil }.count ?? 0) unsynced photos and their local files after stopping queued uploads."
        )
      } else {
        Text(
          "Website deletion remains pending until the server confirms it. Sources are kept while upload or deletion outcomes are uncertain."
        )
      }
    }
    .task { if !AppBootstrap.isUITesting { await model.refreshOwnerList() } }
  }

  private var deletingIsLocalOnly: Bool {
    guard let deleting else { return false }
    return deleting.serverId == nil
      && model.workspace?.operations.filter { $0.voyageId == deleting.id }.allSatisfy {
        $0.request == nil
      } == true
  }

  private var voyages: [LocalVoyage] {
    model.localVoyages.filter { voyage in
      (search.isEmpty || voyage.fields.title.localizedCaseInsensitiveContains(search))
        && (filter == "All" || (filter == "Public") == voyage.confirmedPublished)
    }
  }
  private func status(_ voyage: LocalVoyage) -> String {
    if model.workspace?.operations.contains(where: {
      $0.voyageId == voyage.id && ![.applied, .canceled].contains($0.state)
    }) == true {
      return "Changes waiting to sync"
    }
    return voyage.serverId == nil ? "Saved on this iPhone" : "Uploaded"
  }
}
