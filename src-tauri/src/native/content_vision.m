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
static const VNConfidence LTMinClassificationConfidence = 0.05;
static atomic_bool LTBusy = false;

bool lumetrace_vision_available(void) {
    if (@available(macOS 10.15, *)) return true;
    return false;
}

@interface LTRecognitionJob : NSObject
@property(atomic) BOOL cancelled;
@property(atomic, copy) NSArray<VNRequest *> *requests;
@property(nonatomic) NSUInteger geometryCount;
@end
@implementation LTRecognitionJob
@end

// All persisted rectangles use the displayed image/page: normalized, top-left.
static NSDictionary *LTBox(CGRect box) {
    box = CGRectIntersection(box, CGRectMake(0, 0, 1, 1));
    if (CGRectIsNull(box) || CGRectIsEmpty(box)) return @{@"x":@0,@"y":@0,@"width":@0,@"height":@0};
    return @{@"x":@(box.origin.x), @"y":@(1-CGRectGetMaxY(box)), @"width":@(box.size.width), @"height":@(box.size.height)};
}

static NSDictionary *LTPDFBox(NSRect bounds, CGAffineTransform transform, CGSize size) {
    CGRect box = CGRectApplyAffineTransform(bounds, transform);
    return LTBox(CGRectMake(box.origin.x/size.width, box.origin.y/size.height, box.size.width/size.width, box.size.height/size.height));
}

static BOOL LTRepeatedLine(NSDictionary *line, NSArray *nativeLines) {
    NSString *(^fold)(NSString *) = ^NSString *(NSString *text) {
        return [[[text componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet] componentsJoinedByString:@""] lowercaseString];
    };
    NSDictionary *r = line[@"rect"];
    CGRect box = CGRectMake([r[@"x"] doubleValue], [r[@"y"] doubleValue], [r[@"width"] doubleValue], [r[@"height"] doubleValue]);
    for (NSDictionary *other in nativeLines) {
        if (![fold(line[@"text"]) isEqual:fold(other[@"text"])]) continue;
        NSDictionary *s = other[@"rect"];
        CGRect native = CGRectMake([s[@"x"] doubleValue], [s[@"y"] doubleValue], [s[@"width"] doubleValue], [s[@"height"] doubleValue]);
        CGRect overlap = CGRectIntersection(box, native);
        if (!CGRectIsNull(overlap) && overlap.size.width*overlap.size.height > box.size.width*box.size.height*0.4) return YES;
    }
    return NO;
}

// A page with ordinary text can still contain raster text in an image or Form.
static BOOL LTResourcesContainImages(CGPDFDictionaryRef resources, NSUInteger depth);
typedef struct { BOOL found; NSUInteger depth; } LTImageProbe;
static void LTInspectXObject(const char *key, CGPDFObjectRef value, void *context) {
    (void)key;
    LTImageProbe *probe = context;
    if (probe->found || probe->depth > 8) return;
    CGPDFStreamRef stream;
    if (!CGPDFObjectGetValue(value, kCGPDFObjectTypeStream, &stream)) return;
    CGPDFDictionaryRef dictionary = CGPDFStreamGetDictionary(stream);
    const char *subtype = NULL;
    if (CGPDFDictionaryGetName(dictionary, "Subtype", &subtype) && !strcmp(subtype, "Image")) probe->found = YES;
    CGPDFDictionaryRef nested;
    if (CGPDFDictionaryGetDictionary(dictionary, "Resources", &nested) && LTResourcesContainImages(nested, probe->depth+1)) probe->found = YES;
}
static BOOL LTResourcesContainImages(CGPDFDictionaryRef resources, NSUInteger depth) {
    if (depth > 8) return NO;
    CGPDFDictionaryRef objects;
    if (!CGPDFDictionaryGetDictionary(resources, "XObject", &objects)) return NO;
    LTImageProbe probe = { NO, depth };
    CGPDFDictionaryApplyFunction(objects, LTInspectXObject, &probe);
    return probe.found;
}
static void LTInlineImage(CGPDFScannerRef scanner, void *context) { (void)scanner; *((BOOL *)context) = YES; }
static BOOL LTPageContainsImages(CGPDFPageRef page) {
    CGPDFDictionaryRef dictionary = CGPDFPageGetDictionary(page), resources;
    for (NSUInteger depth=0; dictionary && depth<16; depth++) {
        if (CGPDFDictionaryGetDictionary(dictionary, "Resources", &resources)) {
            if (LTResourcesContainImages(resources, 0)) return YES;
            break;
        }
        if (!CGPDFDictionaryGetDictionary(dictionary, "Parent", &dictionary)) break;
    }
    BOOL inlineImage = NO;
    CGPDFContentStreamRef content = CGPDFContentStreamCreateWithPage(page);
    CGPDFOperatorTableRef table = CGPDFOperatorTableCreate();
    CGPDFOperatorTableSetCallback(table, "BI", LTInlineImage);
    CGPDFScannerRef scanner = CGPDFScannerCreate(content, table, &inlineImage);
    CGPDFScannerScan(scanner);
    CGPDFScannerRelease(scanner); CGPDFOperatorTableRelease(table); CGPDFContentStreamRelease(content);
    return inlineImage;
}

static void LTAppendLabels(NSMutableArray *target, NSArray *source) {
    if (!target || !source) return;
    for (NSDictionary *item in source) {
        if (target.count >= 8) break;
        NSString *identifier = item[@"identifier"];
        NSNumber *confidence = item[@"confidence"];
        if (![identifier isKindOfClass:NSString.class] || ![confidence isKindOfClass:NSNumber.class]) continue;
        BOOL duplicate = NO;
        for (NSDictionary *existing in target) {
            if ([existing[@"identifier"] isEqual:identifier]) { duplicate = YES; break; }
        }
        if (!duplicate) [target addObject:item];
    }
}

API_AVAILABLE(macos(10.15))
static NSString *LTReadImage(CGImageRef image, LTRecognitionJob *job, NSMutableArray *labels, NSMutableArray *geometry, NSError **error) {
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
        if (job.cancelled) return nil;
        if (candidate.string.length) {
            [lines addObject:candidate.string];
            NSMutableArray *words = [NSMutableArray new];
            // Keep exact substring geometry when Vision can provide it. Spaces
            // and unavailable character boxes fall back to the recognized line.
            [candidate.string enumerateSubstringsInRange:NSMakeRange(0, candidate.string.length)
                options:NSStringEnumerationByComposedCharacterSequences usingBlock:^(NSString *s, NSRange range, NSRange enclosing, BOOL *stop) {
                (void)enclosing;
                if (job.cancelled || job.geometryCount >= 50000) { *stop=YES; return; }
                if (![s stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].length) return;
                VNRectangleObservation *word = [candidate boundingBoxForRange:range error:nil];
                if (word) { [words addObject:@{@"start":@(range.location),@"end":@(NSMaxRange(range)),@"rect":LTBox(word.boundingBox)}]; job.geometryCount++; }
            }];
            if (geometry.count < 10000) [geometry addObject:@{@"text":candidate.string,@"rect":LTBox(line.boundingBox),@"words":words}];
        }
    }
    for (VNClassificationObservation *item in classification.results) {
        if (item.confidence >= LTMinClassificationConfidence && labels.count < 8)
            [labels addObject:@{@"identifier":item.identifier, @"confidence":@(item.confidence)}];
    }
    job.requests = @[];
    return [lines componentsJoinedByString:@"\n"];
}

API_AVAILABLE(macos(10.15))
static NSDictionary *LTRecognize(NSURL *url, BOOL isPDF, LTRecognitionJob *job) {
    NSMutableString *body = [NSMutableString new];
    NSMutableArray *labels = [NSMutableArray new];
    NSMutableArray *pages = [NSMutableArray new];
    NSUInteger ocrPages = 0;
    NSError *error = nil;
    if (isPDF) {
        PDFDocument *document = [[PDFDocument alloc] initWithURL:url];
        if (!document || document.isLocked) return @{@"error":@"Unable to read PDF or PDF is password protected"};
        for (NSUInteger i = 0; i < document.pageCount; i++) {
            @autoreleasepool {
                if (job.cancelled) return @{@"error":@"Apple Vision recognition timed out"};
                PDFPage *page = [document pageAtIndex:i];
                NSString *text = page.string ?: @"";
                CGPDFPageRef pageRef = page.pageRef;
                if (!pageRef) return @{@"error":@"Unable to read PDF page"};
                CGRect bounds = CGRectIntersection(CGPDFPageGetBoxRect(pageRef, kCGPDFCropBox), CGPDFPageGetBoxRect(pageRef, kCGPDFMediaBox));
                BOOL rotated = labs(page.rotation) % 180 == 90;
                CGFloat displayWidth = rotated ? bounds.size.height : bounds.size.width;
                CGFloat displayHeight = rotated ? bounds.size.width : bounds.size.height;
                CGFloat maximum = MAX(displayWidth, displayHeight);
                if (!isfinite(maximum) || maximum <= 0 || displayWidth <= 0 || displayHeight <= 0) return @{@"error":@"Invalid PDF page dimensions"};
                CGFloat scale = MIN(2.0, LTMaxDimension / maximum);
                size_t width = MAX(1, ceil(displayWidth * scale)), height = MAX(1, ceil(displayHeight * scale));
                CGSize size = CGSizeMake(width,height);
                // CoreGraphics only scales DOWN in GetDrawingTransform. Asking
                // it for a 2x target merely centers a 1x page with padding.
                // Resolve rotation/crop at 1x, then apply explicit raster scale.
                CGAffineTransform baseTransform = CGPDFPageGetDrawingTransform(pageRef, kCGPDFCropBox, CGRectMake(0,0,displayWidth,displayHeight), 0, true);
                CGAffineTransform transform = CGAffineTransformConcat(baseTransform, CGAffineTransformMakeScale(width/displayWidth,height/displayHeight));
                NSMutableArray *geometry = [NSMutableArray new];
                NSMutableArray *pageLabels = [NSMutableArray new];
                [text enumerateSubstringsInRange:NSMakeRange(0,text.length) options:NSStringEnumerationByLines usingBlock:^(NSString *line, NSRange range, NSRange enclosing, BOOL *stop) {
                    (void)enclosing;
                    if (job.cancelled || geometry.count >= 10000) { *stop=YES; return; }
                    PDFSelection *selection = [page selectionForRange:range];
                    if (!selection || !line.length) return;
                    NSMutableArray *words = [NSMutableArray new];
                    [line enumerateSubstringsInRange:NSMakeRange(0,line.length) options:NSStringEnumerationByComposedCharacterSequences usingBlock:^(NSString *s, NSRange wordRange, NSRange ignored, BOOL *wordStop) {
                        (void)ignored;
                        if (job.cancelled || job.geometryCount >= 50000) { *wordStop=YES; return; }
                        if (![s stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet].length) return;
                        PDFSelection *word = [page selectionForRange:NSMakeRange(range.location+wordRange.location,wordRange.length)];
                        if (word) { [words addObject:@{@"start":@(wordRange.location),@"end":@(NSMaxRange(wordRange)),@"rect":LTPDFBox([word boundsForPage:page],transform,size)}]; job.geometryCount++; }
                    }];
                    [geometry addObject:@{@"text":line,@"rect":LTPDFBox([selection boundsForPage:page],transform,size),@"words":words}];
                }];
                NSString *trimmed = [text stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                NSMutableString *pageBody = [trimmed mutableCopy];
                if (!trimmed.length || LTPageContainsImages(pageRef)) {
                    CGColorSpaceRef colors = CGColorSpaceCreateDeviceRGB();
                    CGContextRef context = CGBitmapContextCreate(NULL, width, height, 8, width * 4, colors, (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                    CGColorSpaceRelease(colors);
                    if (!context) return @{@"error":@"Unable to render PDF page for OCR"};
                    CGContextSetRGBFillColor(context, 1, 1, 1, 1);
                    CGContextFillRect(context, CGRectMake(0, 0, width, height));
                    CGContextConcatCTM(context, transform);
                    CGContextDrawPDFPage(context, pageRef);
                    CGImageRef image = CGBitmapContextCreateImage(context);
                    CGContextRelease(context);
                    if (!image) return @{@"error":@"Unable to render PDF page for OCR"};
                    NSMutableArray *ocrGeometry = [NSMutableArray new];
                    NSString *ocrText = LTReadImage(image, job, pageLabels, ocrGeometry, &error);
                    CGImageRelease(image);
                    ocrPages++;
                    if (!ocrText) return @{@"error":error.localizedDescription ?: @"Apple Vision OCR failed or was cancelled"};
                    LTAppendLabels(labels, pageLabels);
                    NSArray *nativeLines = [geometry copy];
                    for (NSDictionary *line in ocrGeometry) {
                        if (LTRepeatedLine(line,nativeLines)) continue;
                        [pageBody appendFormat:@"%@%@",pageBody.length ? @"\n" : @"",line[@"text"]];
                        [geometry addObject:line];
                    }
                }
                [pages addObject:@{@"number":@(i+1),@"width":@(displayWidth),@"height":@(displayHeight),@"lines":geometry,@"labels":pageLabels}];
                if (pageBody.length) [body appendFormat:@"%@[Page %lu]\n%@", body.length ? @"\n\n" : @"", (unsigned long)i + 1, pageBody];
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
        NSMutableArray *geometry = [NSMutableArray new];
        NSString *text = LTReadImage(image, job, labels, geometry, &error);
        [pages addObject:@{@"number":@1,@"width":@(CGImageGetWidth(image)),@"height":@(CGImageGetHeight(image)),@"lines":geometry,@"labels":labels}];
        CGImageRelease(image);
        if (!text) return @{@"error":error.localizedDescription ?: @"Apple Vision recognition failed or was cancelled"};
        [body appendString:text];
        ocrPages = 1;
    }
    return @{@"body":body, @"labels":labels, @"ocr_pages":@(ocrPages), @"pages":pages};
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
