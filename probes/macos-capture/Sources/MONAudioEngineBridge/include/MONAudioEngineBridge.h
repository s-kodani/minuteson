#import <AVFoundation/AVFoundation.h>

NS_ASSUME_NONNULL_BEGIN

// Keep framework exceptions inside Objective-C; none may unwind Swift frames.
NSError * _Nullable MONInstallAudioTap(
    AVAudioNode *node, AVAudioFormat *format, AVAudioNodeTapBlock block);

NS_ASSUME_NONNULL_END
