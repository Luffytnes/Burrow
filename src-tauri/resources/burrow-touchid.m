// SPDX-License-Identifier: MIT
// Small LocalAuthentication helper bundled with Burrow.

#import <Foundation/Foundation.h>
#import <LocalAuthentication/LocalAuthentication.h>

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        const BOOL checkOnly = argc > 1 && strcmp(argv[1], "--check") == 0;
        NSString *reason = @"Burrow demande votre autorisation";
        if (!checkOnly && argc > 1) {
            NSString *argument = [NSString stringWithUTF8String:argv[1]];
            if (argument != nil && argument.length > 0) {
                reason = argument;
            }
        }

        LAContext *context = [[LAContext alloc] init];
        NSError *availabilityError = nil;
        if (![context canEvaluatePolicy:LAPolicyDeviceOwnerAuthenticationWithBiometrics
                                   error:&availabilityError]) {
            return 2;
        }
        if (checkOnly) {
            return 0;
        }

        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block BOOL authenticated = NO;
        [context evaluatePolicy:LAPolicyDeviceOwnerAuthenticationWithBiometrics
                localizedReason:reason
                          reply:^(BOOL success, NSError *error) {
                              (void)error;
                              authenticated = success;
                              dispatch_semaphore_signal(semaphore);
                          }];
        const dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC);
        if (dispatch_semaphore_wait(semaphore, timeout) != 0) {
            return 3;
        }
        return authenticated ? 0 : 1;
    }
}
