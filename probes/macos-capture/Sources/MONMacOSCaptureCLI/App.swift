import MONMacOSCapture

@main
enum Main {
  static func main() async {
    await CaptureCommand.run()
  }
}
