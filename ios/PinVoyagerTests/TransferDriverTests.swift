import Foundation
import PinVoyagerData
import PinVoyagerDomain
import UIKit
import XCTest

@testable import PinVoyager

@MainActor final class TransferDriverTests: XCTestCase {
  func testWakeCompletionCanRegisterAfterItsEventsHaveAlreadyFinished() async throws {
    let driver = SystemTransferDriver()
    let descriptor = TransferDescriptor(accountId: UUID(), generation: 1, operationId: UUID())
    let saved = CheckpointMarker()
    driver.onCompletion = { _ in await MainActor.run { saved.checkpointed = true } }
    driver.completed(descriptor, identifier: descriptor.sessionIdentifier)
    await driver.finishEvents(descriptor.sessionIdentifier)
    let finished = expectation(description: "Late wake completion is delivered")
    driver.reconnect(descriptor.sessionIdentifier) {
      XCTAssertTrue(saved.checkpointed)
      finished.fulfill()
    }
    await fulfillment(of: [finished], timeout: 1)
  }
}
@MainActor private final class CheckpointMarker { var checkpointed = false }

extension TransferDriverTests {
  func testSystemResponseEvidenceSurvivesANewDriverUntilTheCheckpointIsAcknowledged() async throws {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let files = TransferEvidenceFiles(root: root)
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    let task = session.dataTask(with: URL(string: "https://localhost/test")!)
    let descriptor = TransferDescriptor(accountId: UUID(), generation: 4, operationId: UUID())
    task.taskDescription = descriptor.rawValue
    let received = expectation(description: "Response evidence is written before callback delivery")
    let delegate = TransferDelegate(
      evidence: files,
      completed: { result in
        XCTAssertEqual(result.transportError, .timedOut)
        received.fulfill()
      }, finished: {})
    delegate.urlSession(session, dataTask: task, didReceive: Data("persisted response".utf8))
    delegate.urlSession(session, task: task, didCompleteWithError: URLError(.timedOut))
    await fulfillment(of: [received], timeout: 2)
    let driver = SystemTransferDriver(evidenceRoot: root)
    let pending = try await driver.pendingCompletions(
      sessionIdentifier: descriptor.sessionIdentifier)
    XCTAssertEqual(pending.count, 1)
    XCTAssertEqual(pending.first?.descriptor, descriptor)
    XCTAssertEqual(pending.first?.transportError, .timedOut)
    XCTAssertEqual(
      try Data(contentsOf: files.url(descriptor, taskId: task.taskIdentifier)),
      Data("persisted response".utf8))
    await driver.discardCompletion(try XCTUnwrap(pending.first))
    let acknowledged = try await driver.pendingCompletions(
      sessionIdentifier: descriptor.sessionIdentifier)
    XCTAssertTrue(acknowledged.isEmpty)
  }
}

extension TransferDriverTests {
  func testEvidenceBoundsBodiesAndReconstructsHTTPFailures() async throws {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let files = TransferEvidenceFiles(root: root)
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    let task = session.dataTask(with: URL(string: "https://localhost/test")!)
    let descriptor = TransferDescriptor(accountId: UUID(), generation: 0, operationId: UUID())
    task.taskDescription = descriptor.rawValue
    let delegate = TransferDelegate(evidence: files, completed: { _ in }, finished: {})
    delegate.urlSession(session, dataTask: task, didReceive: Data(repeating: 65, count: 2_097_152))
    delegate.urlSession(session, dataTask: task, didReceive: Data([66]))
    let path = files.url(descriptor, taskId: task.taskIdentifier)
    XCTAssertEqual(try Data(contentsOf: path).count, 1_048_576)
    try APIJSON.encode(
      TransferEvidenceRecord(
        status: 429, requestId: "request-reference", retryAfter: "120", transportError: nil)
    ).write(to: path.appendingPathExtension("json"))
    let completion = try XCTUnwrap(
      try files.completions(sessionIdentifier: descriptor.sessionIdentifier).first)
    XCTAssertEqual(completion.response?.status, 429)
    XCTAssertEqual(completion.response?.headers["retry-after"], "120")
    XCTAssertEqual(completion.response?.headers["x-request-id"], "request-reference")
    XCTAssertEqual(completion.response?.body.count, 1_048_576)
    try Data("damaged".utf8).write(
      to: path.deletingLastPathComponent().appendingPathComponent("2.json"))
    XCTAssertEqual(try files.completions(sessionIdentifier: descriptor.sessionIdentifier).count, 1)
    XCTAssertTrue(try files.completions(sessionIdentifier: "unknown-session").isEmpty)
  }

  func testBackgroundDriverEnumeratesResumesAndCancelsOnlySelectedTasks() async throws {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let body = root.appendingPathComponent("body")
    try Data("file-backed upload".utf8).write(to: body)
    let driver = SystemTransferDriver(evidenceRoot: root)
    let account = UUID()
    let first = TransferDescriptor(accountId: account, generation: 0, operationId: UUID())
    let second = TransferDescriptor(accountId: account, generation: 0, operationId: UUID())
    var policy = SyncPolicy()
    policy.wifiOnly = false
    policy.allowsConstrained = true
    await driver.configure(sessionIdentifier: first.sessionIdentifier, policy: policy)
    var request = URLRequest(url: URL(string: "https://127.0.0.1:9/test")!)
    request.httpMethod = "POST"
    let one = try await driver.createSuspended(
      sessionIdentifier: first.sessionIdentifier, request: request, body: body, descriptor: first,
      policy: policy)
    let two = try await driver.createSuspended(
      sessionIdentifier: first.sessionIdentifier, request: request, body: body, descriptor: second,
      policy: policy)
    let before = await driver.tasks(sessionIdentifier: first.sessionIdentifier)
    XCTAssertEqual(Set(before.map(\.id)), Set([one.id, two.id]))
    await driver.cancel(one, sessionIdentifier: first.sessionIdentifier)
    await driver.resume(two, sessionIdentifier: first.sessionIdentifier)
    await driver.cancel(sessionIdentifier: first.sessionIdentifier)
  }
}

private final class TransferResponseProtocol: URLProtocol, @unchecked Sendable {
  override class func canInit(with request: URLRequest) -> Bool {
    request.url?.host == "transfer.invalid"
  }
  override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
  override func startLoading() {
    let response = HTTPURLResponse(
      url: request.url!, statusCode: 503, httpVersion: "HTTP/1.1",
      headerFields: [
        "Content-Type": "application/json", "Retry-After": "120",
        "x-request-id": "transfer-reference",
      ])!
    client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
    client?.urlProtocol(self, didLoad: Data("{\"message\":\"Try later\"}".utf8))
    client?.urlProtocolDidFinishLoading(self)
  }
  override func stopLoading() {}
}

extension TransferDriverTests {
  func testRealURLSessionDelegatePersistsHTTPStatusAndHeadersBeforeDeliveringCompletion()
    async throws
  {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let evidence = TransferEvidenceFiles(root: root)
    let descriptor = TransferDescriptor(accountId: UUID(), generation: 2, operationId: UUID())
    let completed = expectation(description: "HTTP response has durable evidence")
    let delegate = TransferDelegate(
      evidence: evidence,
      completed: { result in
        XCTAssertEqual(result.response?.status, 503)
        XCTAssertEqual(result.response?.headers["retry-after"], "120")
        XCTAssertEqual(result.response?.headers["x-request-id"], "transfer-reference")
        XCTAssertEqual(result.response?.body, Data("{\"message\":\"Try later\"}".utf8))
        XCTAssertNil(result.transportError)
        let saved = try? evidence.completions(sessionIdentifier: descriptor.sessionIdentifier)
        XCTAssertEqual(saved?.first?.response?.status, 503)
        completed.fulfill()
      }, finished: {})
    let configuration = URLSessionConfiguration.ephemeral
    configuration.protocolClasses = [TransferResponseProtocol.self]
    let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
    defer { session.invalidateAndCancel() }
    let task = session.dataTask(with: URL(string: "https://transfer.invalid/upload")!)
    task.taskDescription = descriptor.rawValue
    task.resume()
    await fulfillment(of: [completed], timeout: 3)
  }
}

extension TransferDriverTests {
  func testCellularAndOrdinarySessionsKeepSeparateResponseEvidenceEvenForTheSameTaskNumber() throws
  {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let files = TransferEvidenceFiles(root: root)
    let account = UUID()
    let operation = UUID()
    let ordinary = TransferDescriptor(accountId: account, generation: 2, operationId: operation)
    let cellular = TransferDescriptor(
      accountId: account, generation: 2, operationId: operation, cellularException: true)
    for (descriptor, status) in [(ordinary, 429), (cellular, 201)] {
      let path = files.url(descriptor, taskId: 1)
      try FileManager.default.createDirectory(
        at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
      try APIJSON.encode(
        TransferEvidenceRecord(status: status, requestId: nil, retryAfter: nil, transportError: nil)
      )
      .write(to: path.appendingPathExtension("json"))
    }
    let first = try XCTUnwrap(
      files.completions(sessionIdentifier: ordinary.sessionIdentifier).first)
    let second = try XCTUnwrap(
      files.completions(sessionIdentifier: cellular.sessionIdentifier).first)
    XCTAssertEqual(first.response?.status, 429)
    XCTAssertEqual(second.response?.status, 201)
    XCTAssertEqual(second.descriptor, cellular)
    files.discard(second)
    XCTAssertTrue(try files.completions(sessionIdentifier: cellular.sessionIdentifier).isEmpty)
    XCTAssertEqual(try files.completions(sessionIdentifier: ordinary.sessionIdentifier).count, 1)
  }
}

extension TransferDriverTests {
  func testBackgroundWakeAcknowledgesUnknownAndSavedAccountsWithoutSelectingAnotherWorkspace()
    async throws
  {
    await AppBootstrap.shared.start()
    let model = try XCTUnwrap(AppBootstrap.shared.model)
    let state = unlockedWorkspace()
    try await model.store.insert(state)
    let previousAccount = model.activeAccountId
    for identifier in [
      "unknown-system-session", TransferSession(accountId: UUID(), generation: 0).identifier,
      TransferSession(accountId: state.account.id, generation: 0, cellularException: true)
        .identifier,
    ] {
      let completed = expectation(description: "Wake events finish after restoration")
      if TransferSession(identifier: identifier) != nil {
        await SystemTransferDriver.shared.finishEvents(identifier)
      }
      AppDelegate().application(
        UIApplication.shared, handleEventsForBackgroundURLSession: identifier
      ) { completed.fulfill() }
      await fulfillment(of: [completed], timeout: 3)
      XCTAssertEqual(model.activeAccountId, previousAccount)
    }
    try await model.store.remove(accountId: state.account.id)
  }
}

extension TransferDriverTests {
  func testADelegateStorageFailureStillReportsAnUnknownTransportFailureForReceiptRecovery()
    async throws
  {
    let root = URL.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try Data("This path cannot become an evidence directory".utf8).write(to: root)
    defer { try? FileManager.default.removeItem(at: root) }
    let session = URLSession(configuration: .ephemeral)
    defer { session.invalidateAndCancel() }
    let task = session.dataTask(with: URL(string: "https://localhost/test")!)
    task.taskDescription =
      TransferDescriptor(accountId: UUID(), generation: 0, operationId: UUID()).rawValue
    let completed = expectation(description: "Failure remains available for receipt lookup")
    let delegate = TransferDelegate(
      evidence: TransferEvidenceFiles(root: root),
      completed: { result in
        XCTAssertEqual(result.transportError, .unknown)
        XCTAssertNil(result.response)
        completed.fulfill()
      }, finished: {})
    delegate.urlSession(session, dataTask: task, didReceive: Data("diagnostic bytes".utf8))
    delegate.urlSession(session, task: task, didCompleteWithError: CocoaError(.fileReadCorruptFile))
    await fulfillment(of: [completed], timeout: 2)
  }
  func testNetworkObserverDeliversAnInitialPathOnTheMainActor() async {
    let changed = expectation(description: "Initial system network path")
    var first = true
    let observer = NetworkObserver { _ in
      MainActor.assertIsolated()
      if first {
        first = false
        changed.fulfill()
      }
    }
    await fulfillment(of: [changed], timeout: 3)
    withExtendedLifetime(observer) {}
  }
}
