#import <AppKit/AppKit.h>
#import <ImageIO/ImageIO.h>

static const CGFloat kArtworkScale = 0.8125;
static const CGFloat kCornerRadiusScale = 0.224;

static void Fail(NSString *message) {
    fprintf(stderr, "error: %s\n", message.UTF8String);
    exit(EXIT_FAILURE);
}

static CGImageRef CreateIcon(CGImageRef source, size_t size) {
    CGColorSpaceRef colorSpace = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(
        NULL,
        size,
        size,
        8,
        0,
        colorSpace,
        (CGBitmapInfo)kCGImageAlphaPremultipliedLast
    );
    CGColorSpaceRelease(colorSpace);
    if (context == NULL) return NULL;

    CGContextSetInterpolationQuality(context, kCGInterpolationHigh);
    const CGFloat artworkSize = (CGFloat)size * kArtworkScale;
    const CGFloat inset = ((CGFloat)size - artworkSize) / 2.0;
    const CGRect artworkRect = CGRectMake(inset, inset, artworkSize, artworkSize);
    CGPathRef mask = CGPathCreateWithRoundedRect(
        artworkRect,
        artworkSize * kCornerRadiusScale,
        artworkSize * kCornerRadiusScale,
        NULL
    );
    CGContextAddPath(context, mask);
    CGContextClip(context);
    CGContextDrawImage(context, artworkRect, source);
    CGPathRelease(mask);

    CGImageRef icon = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    return icon;
}

static BOOL WritePNG(CGImageRef image, NSURL *destinationURL) {
    CGImageDestinationRef destination = CGImageDestinationCreateWithURL(
        (__bridge CFURLRef)destinationURL,
        CFSTR("public.png"),
        1,
        NULL
    );
    if (destination == NULL) return NO;
    CGImageDestinationAddImage(destination, image, NULL);
    const BOOL written = CGImageDestinationFinalize(destination);
    CFRelease(destination);
    return written;
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 4) {
            Fail(@"usage: generate_macos_icon SOURCE_PNG OUTPUT_ICNS PREVIEW_PNG");
        }

        NSFileManager *fileManager = NSFileManager.defaultManager;
        NSURL *sourceURL = [NSURL fileURLWithPath:@(argv[1])];
        NSURL *outputURL = [NSURL fileURLWithPath:@(argv[2])];
        NSURL *previewURL = [NSURL fileURLWithPath:@(argv[3])];
        CGImageSourceRef imageSource = CGImageSourceCreateWithURL(
            (__bridge CFURLRef)sourceURL,
            NULL
        );
        if (imageSource == NULL) Fail(@"source PNG introuvable ou invalide");
        CGImageRef source = CGImageSourceCreateImageAtIndex(imageSource, 0, NULL);
        CFRelease(imageSource);
        if (source == NULL) Fail(@"impossible de décoder la source PNG");

        NSString *temporaryName = [NSString stringWithFormat:
            @"burrow-icon-%@", NSUUID.UUID.UUIDString
        ];
        NSURL *temporaryRoot = [NSURL fileURLWithPath:NSTemporaryDirectory() isDirectory:YES];
        temporaryRoot = [temporaryRoot URLByAppendingPathComponent:temporaryName isDirectory:YES];
        NSURL *generatedURL = [temporaryRoot
            URLByAppendingPathComponent:@"generated"
            isDirectory:YES
        ];
        NSError *error = nil;
        if (![fileManager createDirectoryAtURL:generatedURL
                   withIntermediateDirectories:YES
                                    attributes:nil
                                         error:&error]) {
            CGImageRelease(source);
            Fail(error.localizedDescription);
        }

        CGImageRef preview = CreateIcon(source, 1024);
        CGImageRelease(source);
        if (preview == NULL || !WritePNG(preview, previewURL)) {
            if (preview != NULL) CGImageRelease(preview);
            [fileManager removeItemAtURL:temporaryRoot error:nil];
            Fail(@"impossible de générer l’aperçu 1024x1024");
        }
        CGImageRelease(preview);

        [fileManager createDirectoryAtURL:outputURL.URLByDeletingLastPathComponent
              withIntermediateDirectories:YES
                               attributes:nil
                                    error:&error];
        NSTask *tauriIcon = [[NSTask alloc] init];
        tauriIcon.executableURL = [NSURL fileURLWithPath:@"/usr/bin/env"];
        tauriIcon.arguments = @[
            @"npx", @"tauri", @"icon", previewURL.path, @"--output", generatedURL.path
        ];
        if (![tauriIcon launchAndReturnError:&error]) {
            [fileManager removeItemAtURL:temporaryRoot error:nil];
            Fail(error.localizedDescription);
        }
        [tauriIcon waitUntilExit];
        if (tauriIcon.terminationStatus != 0) {
            [fileManager removeItemAtURL:temporaryRoot error:nil];
            Fail([NSString stringWithFormat:
                @"tauri icon a quitté avec le code %d", tauriIcon.terminationStatus
            ]);
        }

        NSURL *generatedICNS = [generatedURL URLByAppendingPathComponent:@"icon.icns"];
        NSURL *stagedICNS = [outputURL.URLByDeletingLastPathComponent
            URLByAppendingPathComponent:[NSString stringWithFormat:
                @".%@.%@.tmp", outputURL.lastPathComponent, NSUUID.UUID.UUIDString
            ]
        ];
        if (![fileManager copyItemAtURL:generatedICNS toURL:stagedICNS error:&error]) {
            [fileManager removeItemAtURL:temporaryRoot error:nil];
            Fail(error.localizedDescription);
        }
        if ([fileManager fileExistsAtPath:outputURL.path]) {
            if (![fileManager replaceItemAtURL:outputURL
                                 withItemAtURL:stagedICNS
                                backupItemName:nil
                                       options:0
                              resultingItemURL:nil
                                         error:&error]) {
                [fileManager removeItemAtURL:stagedICNS error:nil];
                [fileManager removeItemAtURL:temporaryRoot error:nil];
                Fail(error.localizedDescription);
            }
        } else if (![fileManager moveItemAtURL:stagedICNS toURL:outputURL error:&error]) {
            [fileManager removeItemAtURL:stagedICNS error:nil];
            [fileManager removeItemAtURL:temporaryRoot error:nil];
            Fail(error.localizedDescription);
        }
        [fileManager removeItemAtURL:temporaryRoot error:nil];

        printf("Icône macOS générée: %s\n", outputURL.path.UTF8String);
    }
    return EXIT_SUCCESS;
}
