// System frameworks only. Caller supplies an authorized local file; originals
// remain read-only. Each image/page has bounded raster size and releases memory.
#import <Foundation/Foundation.h>
#import <Vision/Vision.h>
#import <PDFKit/PDFKit.h>
#import <ImageIO/ImageIO.h>
#include <stdatomic.h>
#include <stdbool.h>

static const size_t LTMaxDimension = 3200;
static const NSUInteger LTMaxCharacters = 5000000;
static atomic_bool LTBusy = false;

bool lumetrace_vision_available(void) {
    if (@available(macOS 10.15, *)) return true;
    return false;
}

@interface LTRecognitionJob : NSObject
@property(atomic) BOOL cancelled;
@property(atomic, copy) NSArray<VNRequest *> *requests;
@end
@implementation LTRecognitionJob
@end

API_AVAILABLE(macos(10.15))
static NSString *LTReadImage(CGImageRef image, LTRecognitionJob *job, NSMutableArray *labels, NSError **error) {
    if (job.cancelled) return nil;
    VNRecognizeTextRequest *ocr = [VNRecognizeTextRequest new];
    ocr.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    ocr.usesLanguageCorrection = YES;
    ocr.preferBackgroundProcessing = YES;
    if (@available(macOS 13.0, *)) ocr.automaticallyDetectsLanguage = YES;
    NSArray *supported;
    if (@available(macOS 12.0, *)) supported = [ocr supportedRecognitionLanguagesAndReturnError:error];
    else supported = [VNRecognizeTextRequest supportedRecognitionLanguagesForTextRecognitionLevel:ocr.recognitionLevel revision:ocr.revision error:error];
    if (!supported) return nil;
    NSMutableArray *languages = [NSMutableArray new];
    for (NSString *language in @[@"zh-Hans", @"zh-Hant", @"en-US", @"ja-JP", @"ko-KR", @"de-DE", @"fr-FR", @"es-ES"])
        if ([supported containsObject:language]) [languages addObject:language];
    ocr.recognitionLanguages = languages;
    VNClassifyImageRequest *classification = nil;
    if (labels) {
        classification = [VNClassifyImageRequest new];
        classification.preferBackgroundProcessing = YES;
    }
    job.requests = classification ? @[ocr, classification] : @[ocr];
    if (job.cancelled) return nil;
    VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:image options:@{}];
    if (![handler performRequests:job.requests error:error] || job.cancelled) return nil;
    NSMutableArray *lines = [NSMutableArray new];
    for (VNRecognizedTextObservation *line in ocr.results) {
        VNRecognizedText *candidate = [line topCandidates:1].firstObject;
        if (candidate.string.length) [lines addObject:candidate.string];
    }
    for (VNClassificationObservation *item in classification.results) {
        if (item.confidence >= 0.3 && labels.count < 8)
            [labels addObject:@{@"identifier":item.identifier, @"confidence":@(item.confidence)}];
    }
    job.requests = @[];
    return [lines componentsJoinedByString:@"\n"];
}

API_AVAILABLE(macos(10.15))
static NSDictionary *LTRecognize(NSURL *url, BOOL isPDF, LTRecognitionJob *job) {
    NSMutableString *body = [NSMutableString new];
    NSMutableArray *labels = [NSMutableArray new];
    NSUInteger ocrPages = 0;
    NSError *error = nil;
    if (isPDF) {
        PDFDocument *document = [[PDFDocument alloc] initWithURL:url];
        if (!document || document.isLocked) return @{@"error":@"Unable to read PDF or PDF is password protected"};
        for (NSUInteger i = 0; i < document.pageCount; i++) {
            @autoreleasepool {
                if (job.cancelled) return @{@"error":@"Apple Vision recognition timed out"};
                PDFPage *page = [document pageAtIndex:i];
                NSString *text = [page.string stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                // Mixed PDFs are handled page by page. Existing text is never OCRed.
                if (!text.length) {
                    NSRect bounds = [page boundsForBox:kPDFDisplayBoxMediaBox];
                    CGFloat maximum = MAX(bounds.size.width, bounds.size.height);
                    if (!isfinite(maximum) || maximum <= 0 || bounds.size.width <= 0 || bounds.size.height <= 0)
                        return @{@"error":@"Invalid PDF page dimensions"};
                    CGFloat scale = MIN(2.0, LTMaxDimension / maximum);
                    size_t width = MAX(1, ceil(bounds.size.width * scale));
                    size_t height = MAX(1, ceil(bounds.size.height * scale));
                    CGColorSpaceRef colors = CGColorSpaceCreateDeviceRGB();
                    CGContextRef context = CGBitmapContextCreate(NULL, width, height, 8, width * 4, colors, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                    CGColorSpaceRelease(colors);
                    if (!context) return @{@"error":@"Unable to render PDF page for OCR"};
                    CGContextSetRGBFillColor(context, 1, 1, 1, 1);
                    CGContextFillRect(context, CGRectMake(0, 0, width, height));
                    CGContextScaleCTM(context, scale, scale);
                    CGContextTranslateCTM(context, -bounds.origin.x, -bounds.origin.y);
                    [page drawWithBox:kPDFDisplayBoxMediaBox toContext:context];
                    CGImageRef image = CGBitmapContextCreateImage(context);
                    CGContextRelease(context);
                    if (!image) return @{@"error":@"Unable to render PDF page for OCR"};
                    text = LTReadImage(image, job, nil, &error);
                    CGImageRelease(image);
                    ocrPages++;
                    if (!text) return @{@"error":error.localizedDescription ?: @"Apple Vision OCR failed or was cancelled"};
                }
                if (text.length) [body appendFormat:@"%@[Page %lu]\n%@", body.length ? @"\n\n" : @"", (unsigned long)i + 1, text];
                if (body.length > LTMaxCharacters) break;
            }
        }
    } else {
        CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url, (__bridge CFDictionaryRef)@{(id)kCGImageSourceShouldCache:@NO});
        if (!source) return @{@"error":@"Unable to decode image for recognition"};
        CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)@{
            (id)kCGImageSourceCreateThumbnailFromImageAlways:@YES,
            (id)kCGImageSourceCreateThumbnailWithTransform:@YES,
            (id)kCGImageSourceThumbnailMaxPixelSize:@(LTMaxDimension),
            (id)kCGImageSourceShouldCacheImmediately:@YES
        });
        CFRelease(source);
        if (!image) return @{@"error":@"Unable to decode image for recognition"};
        NSString *text = LTReadImage(image, job, labels, &error);
        CGImageRelease(image);
        if (!text) return @{@"error":error.localizedDescription ?: @"Apple Vision recognition failed or was cancelled"};
        [body appendString:text];
        ocrPages = 1;
    }
    return @{@"body":body, @"labels":labels, @"ocr_pages":@(ocrPages)};
}

char *lumetrace_vision_read(const char *path, bool pdf) {
    @autoreleasepool {
        bool expected = false;
        if (!atomic_compare_exchange_strong(&LTBusy, &expected, true))
            return strdup("{\"error\":\"Apple Vision is still finishing a previous request\"}");
        NSString *name = [[NSFileManager defaultManager] stringWithFileSystemRepresentation:path length:strlen(path)];
        NSURL *url = [NSURL fileURLWithPath:name];
        LTRecognitionJob *job = [LTRecognitionJob new];
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block NSDictionary *result = nil;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
            @autoreleasepool {
                @try {
                    if (@available(macOS 10.15, *)) result = LTRecognize(url, pdf, job);
                    else result = @{@"error":@"Apple Vision text recognition requires macOS 10.15 or later"};
                }
                @catch (NSException *exception) { result = @{@"error":@"Apple Vision could not process this file"}; }
                atomic_store(&LTBusy, false);
                dispatch_semaphore_signal(done);
            }
        });
        // Bound callers even if an OS request stalls. The busy gate remains held
        // by the native job until it exits, so timeouts cannot accumulate workers.
        if (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC))) {
            job.cancelled = YES;
            if (@available(macOS 10.15, *)) for (VNRequest *request in job.requests) [request cancel];
            return strdup("{\"error\":\"Apple Vision recognition timed out; retry from background tasks\"}");
        }
        NSData *json = [NSJSONSerialization dataWithJSONObject:result ?: @{} options:0 error:nil];
        NSString *serialized = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
        return strdup(serialized.UTF8String ?: "{\"error\":\"Apple Vision serialization failed\"}");
    }
}

void lumetrace_vision_free(char *result) { free(result); }
