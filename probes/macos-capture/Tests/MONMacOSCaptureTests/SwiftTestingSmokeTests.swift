import Testing

@testable import MONMacOSCapture

@Test func captureCoreOptionsRemainAvailableToSwiftTestingWithoutStartingCLI() throws {
  let bounded = try Options.parse(["--output", "/tmp/mon-swift-testing-smoke"])
  #expect(bounded.duration == 30)
  #expect(!bounded.stopOnFile)
  let controlled = try Options.parse([
    "--output", "/tmp/mon-swift-testing-smoke", "--stop-on-file", "--parent-pid", "1234",
    "--until-stopped",
  ])
  #expect(controlled.duration == nil)
  #expect(controlled.stopOnFile)
  #expect(controlled.parentPID == 1234)
}
