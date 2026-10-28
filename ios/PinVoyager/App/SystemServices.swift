import BackgroundTasks
import Network
import PinVoyagerData
import PinVoyagerDomain
import UIKit
import UserNotifications

@MainActor
final class NetworkObserver {
  private let monitor = NWPathMonitor()
  init(changed: @escaping @MainActor (NetworkState) -> Void) {
    monitor.pathUpdateHandler = { path in
      let state: NetworkState =
        path.status != .satisfied
        ? .offline
        : path.usesInterfaceType(.cellular)
          ? .cellular(constrained: path.isConstrained) : .wifi(constrained: path.isConstrained)
      Task { @MainActor in changed(state) }
    }
    monitor.start(queue: DispatchQueue(label: "com.pinvoyager.network"))
  }
  deinit { monitor.cancel() }
}

@MainActor
final class SystemTransferDriver: NSObject, TransferDriver {
  static let shared = SystemTransferDriver()
  var onCompletion: (@Sendable (TransferCompletion) async throws -> Void)?
  private let evidence: TransferEvidenceFiles
  init(evidenceRoot: URL = AppBootstrap.directory.appendingPathComponent("files")) {
    evidence = TransferEvidenceFiles(root: evidenceRoot)
    super.init()
  }
  func pendingCompletions(sessionIdentifier: String) async throws -> [TransferCompletion] {
    let evidence = evidence
    return try await Task.detached {
      try evidence.completions(sessionIdentifier: sessionIdentifier)
    }.value
  }
  func discardCompletion(_ completion: TransferCompletion) async {
    let evidence = evidence
    await Task.detached { evidence.discard(completion) }.value
  }
  private var sessions: [String: URLSession] = [:]
  private var handlers: [String: [() -> Void]] = [:]
  private var finishedBatches: Set<String> = []
  private var drains: [String: Task<Void, Never>] = [:]
  private var callbacks: [String: [Task<Void, Never>]] = [:]

  func configure(sessionIdentifier: String, policy: SyncPolicy) {
    _ = session(sessionIdentifier, policy: policy)
  }

  func reconnect(
    _ identifier: String, policy: SyncPolicy = SyncPolicy(), completion: (() -> Void)? = nil
  ) {
    guard TransferSession(identifier: identifier) != nil else {
      completion?()
      return
    }
    if let completion {
      if finishedBatches.remove(identifier) != nil {
        completion()
        return
      }
      handlers[identifier, default: []].append(completion)
    }
    _ = session(identifier, policy: policy)
  }

  private func session(_ identifier: String, policy: SyncPolicy) -> URLSession {
    if let session = sessions[identifier] { return session }
    let configuration = URLSessionConfiguration.background(withIdentifier: identifier)
    configuration.sessionSendsLaunchEvents = true
    configuration.isDiscretionary = false
    configuration.httpMaximumConnectionsPerHost = 2
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.allowsCellularAccess = !policy.wifiOnly
    configuration.allowsExpensiveNetworkAccess = !policy.wifiOnly
    configuration.allowsConstrainedNetworkAccess = policy.allowsConstrained
    configuration.timeoutIntervalForResource = 7 * 24 * 60 * 60
    configuration.timeoutIntervalForRequest = 15 * 60
    let delegate = TransferDelegate(
      evidence: evidence,
      completed: { [weak self] descriptor in self?.completed(descriptor, identifier: identifier) },
      finished: { [weak self] in self?.finished(identifier) })
    let queue = OperationQueue()
    queue.maxConcurrentOperationCount = 1
    queue.name = "com.pinvoyager.transfer-events"
    let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: queue)
    sessions[identifier] = session
    return session
  }

  func tasks(sessionIdentifier: String) async -> [SystemTransfer] {
    let tasks = await session(sessionIdentifier, policy: SyncPolicy()).allTasks
    return tasks.compactMap { task in
      guard let description = task.taskDescription,
        let descriptor = TransferDescriptor(rawValue: description)
      else { return nil }
      return SystemTransfer(id: task.taskIdentifier, descriptor: descriptor)
    }
  }

  func createSuspended(
    sessionIdentifier: String, request: URLRequest, body: URL, descriptor: TransferDescriptor,
    policy: SyncPolicy
  ) async throws -> SystemTransfer {
    let task = session(sessionIdentifier, policy: policy).uploadTask(with: request, fromFile: body)
    task.taskDescription = descriptor.rawValue
    return SystemTransfer(id: task.taskIdentifier, descriptor: descriptor)
  }

  func resume(_ task: SystemTransfer, sessionIdentifier: String) async {
    let tasks = await session(sessionIdentifier, policy: SyncPolicy()).allTasks
    tasks.first { $0.taskIdentifier == task.id }?.resume()
  }

  func cancel(sessionIdentifier: String) async {
    for task in await session(sessionIdentifier, policy: SyncPolicy()).allTasks { task.cancel() }
  }

  func cancel(_ transfer: SystemTransfer, sessionIdentifier: String) async {
    let tasks = await session(sessionIdentifier, policy: SyncPolicy()).allTasks
    tasks.first { $0.taskIdentifier == transfer.id }?.cancel()
  }

  func completed(_ descriptor: TransferDescriptor, identifier: String) {
    completed(TransferCompletion(descriptor: descriptor), identifier: identifier)
  }

  func completed(_ completion: TransferCompletion, identifier: String) {
    let callback = onCompletion
    callbacks[identifier, default: []].append(
      Task {
        do { try await callback?(completion) } catch {
          // The durable evidence remains available for foreground receipt reconciliation.
        }
      })
  }

  private func finished(_ identifier: String) {
    Task { await finishEvents(identifier) }
  }

  func finishEvents(_ identifier: String) async {
    if let active = drains[identifier] {
      await active.value
      return
    }
    let work = Task { await drainEvents(identifier) }
    drains[identifier] = work
    await work.value
  }

  private func drainEvents(_ identifier: String) async {
    defer { drains.removeValue(forKey: identifier) }
    while true {
      let pending = callbacks.removeValue(forKey: identifier) ?? []
      if pending.isEmpty { break }
      for callback in pending { await callback.value }
    }
    if let completions = handlers.removeValue(forKey: identifier) {
      for completion in completions { completion() }
    } else {
      finishedBatches.insert(identifier)
    }
  }

}

struct TransferEvidenceRecord: Codable {
  let status: Int
  let requestId: String?
  let retryAfter: String?
  let transportError: Int?
}

struct TransferEvidenceFiles: Sendable {
  let root: URL
  func url(_ descriptor: TransferDescriptor, taskId: Int) -> URL {
    root.appendingPathComponent(
      "\(descriptor.accountId)/transfer-evidence/\(descriptor.generation)/\(descriptor.cellularException ? "cellular/" : "")\(descriptor.operationId)/\(taskId)"
    )
  }
  func completions(sessionIdentifier: String) throws -> [TransferCompletion] {
    guard let session = TransferSession(identifier: sessionIdentifier) else { return [] }
    let directory = root.appendingPathComponent(
      "\(session.accountId)/transfer-evidence/\(session.generation)\(session.cellularException ? "/cellular" : "")"
    )
    guard FileManager.default.fileExists(atPath: directory.path) else { return [] }
    var results: [TransferCompletion] = []
    for operation in try FileManager.default.contentsOfDirectory(
      at: directory, includingPropertiesForKeys: nil)
    {
      guard let id = UUID(uuidString: operation.lastPathComponent) else { continue }
      for file in try FileManager.default.contentsOfDirectory(
        at: operation, includingPropertiesForKeys: nil)
      {
        guard file.pathExtension == "json",
          let taskId = Int(file.deletingPathExtension().lastPathComponent), taskId > 0,
          let record = try? APIJSON.decoder().decode(
            TransferEvidenceRecord.self, from: read(file, limit: 16384))
        else { continue }
        var headers: [String: String] = [:]
        headers["x-request-id"] = record.requestId
        headers["retry-after"] = record.retryAfter
        let body = (try? read(file.deletingPathExtension(), limit: 1_048_576)) ?? Data()
        results.append(
          TransferCompletion(
            descriptor: TransferDescriptor(
              accountId: session.accountId,
              generation: session.generation, operationId: id,
              cellularException: session.cellularException), taskId: taskId,
            response: record.status > 0
              ? HTTPResponse(status: record.status, headers: headers, body: body) : nil,
            transportError: record.transportError.flatMap { URLError.Code(rawValue: $0) }))
      }
    }
    return results
  }
  func discard(_ completion: TransferCompletion) {
    guard let id = completion.taskId else { return }
    let path = url(completion.descriptor, taskId: id)
    try? FileManager.default.removeItem(at: path.appendingPathExtension("json"))
    try? FileManager.default.removeItem(at: path)
  }
  private func read(_ url: URL, limit: Int) throws -> Data {
    let file = try FileHandle(forReadingFrom: url)
    defer { try? file.close() }
    return try file.read(upToCount: limit) ?? Data()
  }
}

final class TransferDelegate: NSObject, URLSessionDataDelegate, @unchecked Sendable {
  let completed: @MainActor @Sendable (TransferCompletion) -> Void
  let finished: @MainActor @Sendable () -> Void
  let evidence: TransferEvidenceFiles
  init(
    evidence: TransferEvidenceFiles,
    completed: @escaping @MainActor @Sendable (TransferCompletion) -> Void,
    finished: @escaping @MainActor @Sendable () -> Void
  ) {
    self.evidence = evidence
    self.completed = completed
    self.finished = finished
  }
  private func evidenceURL(_ task: URLSessionTask) -> URL? {
    guard let value = task.taskDescription, let descriptor = TransferDescriptor(rawValue: value)
    else { return nil }
    return evidence.url(descriptor, taskId: task.taskIdentifier)
  }
  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    guard let url = evidenceURL(dataTask) else { return }
    do {
      try FileManager.default.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
      if !FileManager.default.fileExists(atPath: url.path) {
        FileManager.default.createFile(atPath: url.path, contents: nil)
      }
      let file = try FileHandle(forWritingTo: url)
      defer { try? file.close() }
      let length = try file.seekToEnd()
      if length < 1_048_576 { try file.write(contentsOf: data.prefix(Int(1_048_576 - length))) }
    } catch {
      // The receipt, rather than this diagnostic response body, determines acceptance.
    }
  }
  func urlSession(
    _ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?
  ) {
    guard let value = task.taskDescription, let descriptor = TransferDescriptor(rawValue: value)
    else { return }
    let response = task.response as? HTTPURLResponse
    let url = evidenceURL(task)
    let body = url.flatMap { try? Data(contentsOf: $0) } ?? Data()
    if let url {
      let record = TransferEvidenceRecord(
        status: response?.statusCode ?? 0,
        requestId: response?.value(forHTTPHeaderField: "x-request-id"),
        retryAfter: response?.value(forHTTPHeaderField: "Retry-After"),
        transportError: (error as? URLError)?.code.rawValue
          ?? (error == nil ? nil : URLError.unknown.rawValue))
      do {
        try FileManager.default.createDirectory(
          at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try APIJSON.encode(record).write(
          to: url.appendingPathExtension("json"),
          options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
      } catch {
        // A missing receipt still leaves the operation uncertain in the outbox.
      }
    }
    let result = TransferCompletion(
      descriptor: descriptor, taskId: task.taskIdentifier,
      response: response.map { response in
        HTTPResponse(
          status: response.statusCode,
          headers: Dictionary(
            response.allHeaderFields.map {
              (String(describing: $0.key), String(describing: $0.value))
            },
            uniquingKeysWith: { _, new in new }), body: body)
      }, transportError: (error as? URLError)?.code ?? (error == nil ? nil : .unknown))
    DispatchQueue.main.async { self.completed(result) }
  }
  func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
    DispatchQueue.main.async { self.finished() }
  }
}

@MainActor
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
  var bootstrap: AppBootstrap { .shared }
  func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
  ) -> Bool {
    UNUserNotificationCenter.current().delegate = self
    BGTaskScheduler.shared.register(forTaskWithIdentifier: "com.pinvoyager.app.sync", using: nil) {
      [weak self] task in
      Task { @MainActor in self?.process(task) }
    }
    return true
  }
  func application(
    _ application: UIApplication, handleEventsForBackgroundURLSession identifier: String,
    completionHandler: @escaping () -> Void
  ) {
    Task {
      await bootstrap.start()
      let policy: SyncPolicy
      if let session = TransferSession(identifier: identifier),
        let store = bootstrap.model?.store
      {
        policy = ((try? await store.read(accountId: session.accountId).policy) ?? SyncPolicy())
          .forUpload(cellularException: session.cellularException)
      } else {
        policy = SyncPolicy()
      }
      SystemTransferDriver.shared.reconnect(
        identifier, policy: policy, completion: completionHandler)
      await bootstrap.model?.activate()
    }
  }
  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification
  ) async -> UNNotificationPresentationOptions {
    [.banner, .list, .sound]
  }

  func schedulePreparation() {
    guard bootstrap.model?.workspace?.policy.backgroundPreparation == true else { return }
    let request = BGProcessingTaskRequest(identifier: "com.pinvoyager.app.sync")
    request.requiresNetworkConnectivity = true
    request.requiresExternalPower = false
    do { try BGTaskScheduler.shared.submit(request) } catch {
      // Foreground and URLSession reconciliation remain available when iOS declines scheduling.
    }
  }
  private func process(_ task: BGTask) {
    let work = Task {
      await bootstrap.start()
      await bootstrap.model?.sync()
      task.setTaskCompleted(success: !Task.isCancelled)
    }
    task.expirationHandler = { work.cancel() }
  }
}
