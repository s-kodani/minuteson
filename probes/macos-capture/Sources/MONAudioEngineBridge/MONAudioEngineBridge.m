#import "MONAudioEngineBridge.h"

NSError *MONInstallAudioTap(AVAudioNode *node, AVAudioFormat *format, AVAudioNodeTapBlock block) {
    @try {
        // The device may change after Swift validates its format. Retire this
        // engine on failure and let the existing bounded rebind path retry.
        [node installTapOnBus:0 bufferSize:1024 format:format block:block];
        return nil;
    } @catch (NSException *exception) {
        return [NSError errorWithDomain:@"jp.mon.capture.audio-engine" code:1 userInfo:@{
            NSLocalizedDescriptionKey: [NSString stringWithFormat:
                @"microphone tap installation failed: %@: %@", exception.name,
                exception.reason ?: @"audio engine configuration changed"]
        }];
    }
}
