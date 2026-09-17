// Generate synthetic OCR fixtures only. Usage: vision-fixtures EMPTY_DIRECTORY
// No network, application state, user documents, or test application identity.
#import <AppKit/AppKit.h>
#import <CoreText/CoreText.h>
#import <ImageIO/ImageIO.h>
#import <PDFKit/PDFKit.h>

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 2) return 2;
        NSString *directory = @(argv[1]);
        if (![[NSFileManager defaultManager] fileExistsAtPath:directory]) return 2;
        NSImage *picture = [[NSImage alloc] initWithSize:NSMakeSize(1400,500)];
        [picture lockFocus];
        [NSColor.whiteColor setFill]; NSRectFill(NSMakeRect(0,0,1400,500));
        [@"ORCHID September 20" drawAtPoint:NSMakePoint(50,300) withAttributes:@{
            NSFontAttributeName:[NSFont systemFontOfSize:60], NSForegroundColorAttributeName:NSColor.blackColor}];
        [@"项目预算 北京 工作计划" drawAtPoint:NSMakePoint(50,170) withAttributes:@{
            NSFontAttributeName:[NSFont systemFontOfSize:56], NSForegroundColorAttributeName:NSColor.blackColor}];
        [picture unlockFocus];
        CGImageRef image = [picture CGImageForProposedRect:NULL context:nil hints:nil];
        for (NSArray *format in @[@[@"document.png", @"public.png"], @[@"document.jpg", @"public.jpeg"], @[@"document.tiff", @"public.tiff"]]) {
            NSURL *url = [NSURL fileURLWithPath:[directory stringByAppendingPathComponent:format[0]]];
            if ([[NSFileManager defaultManager] fileExistsAtPath:url.path]) return 3;
            CGImageDestinationRef destination = CGImageDestinationCreateWithURL((__bridge CFURLRef)url, (__bridge CFStringRef)format[1], 1, NULL);
            CGImageDestinationAddImage(destination, image, NULL);
            BOOL ok = CGImageDestinationFinalize(destination); CFRelease(destination);
            if (!ok) return 4;
        }
        for (NSString *name in @[@"scan.pdf", @"mixed.pdf", @"embedded.pdf"]) {
            NSURL *url = [NSURL fileURLWithPath:[directory stringByAppendingPathComponent:name]];
            if ([[NSFileManager defaultManager] fileExistsAtPath:url.path]) return 3;
            BOOL embedded = [name isEqual:@"embedded.pdf"];
            CGRect box = CGRectMake(0,0,700,embedded ? 500 : 250);
            CGContextRef pdf = CGPDFContextCreateWithURL((__bridge CFURLRef)url, &box, NULL);
            if ([name isEqual:@"mixed.pdf"]) {
                CGPDFContextBeginPage(pdf, NULL);
                CGContextSetTextPosition(pdf, 30, 150);
                CTFontRef font = CTFontCreateWithName(CFSTR("Helvetica"), 24, NULL);
                NSAttributedString *text = [[NSAttributedString alloc] initWithString:@"TEXTLAYER QUARTZ" attributes:@{(__bridge id)kCTFontAttributeName:(__bridge id)font}];
                CTLineRef line = CTLineCreateWithAttributedString((__bridge CFAttributedStringRef)text);
                CTLineDraw(line, pdf); CFRelease(line); CFRelease(font);
                CGPDFContextEndPage(pdf);
            }
            CGPDFContextBeginPage(pdf, NULL);
            if (embedded) {
                CGContextSetTextPosition(pdf, 30, 430);
                CTFontRef font = CTFontCreateWithName(CFSTR("Helvetica"), 24, NULL);
                NSAttributedString *text = [[NSAttributedString alloc] initWithString:@"NATIVE HEADER QUARTZ" attributes:@{(__bridge id)kCTFontAttributeName:(__bridge id)font}];
                CTLineRef line = CTLineCreateWithAttributedString((__bridge CFAttributedStringRef)text);
                CTLineDraw(line,pdf); CFRelease(line); CFRelease(font);
            }
            CGContextDrawImage(pdf, CGRectMake(0,0,700,250), image); CGPDFContextEndPage(pdf);
            CGPDFContextClose(pdf); CGContextRelease(pdf);
        }
        NSURL *rotatedURL = [NSURL fileURLWithPath:[directory stringByAppendingPathComponent:@"rotated.pdf"]];
        if ([[NSFileManager defaultManager] fileExistsAtPath:rotatedURL.path]) return 3;
        PDFDocument *rotated = [[PDFDocument alloc] initWithURL:[NSURL fileURLWithPath:[directory stringByAppendingPathComponent:@"embedded.pdf"]]];
        PDFPage *page = [rotated pageAtIndex:0];
        [page setBounds:NSMakeRect(10,10,680,480) forBox:kPDFDisplayBoxCropBox];
        page.rotation = 90;
        if (![rotated writeToURL:rotatedURL]) return 4;
        puts("Generated synthetic PNG/JPEG/TIFF, scanned PDF, and mixed text/scan PDF.");
    }
    return 0;
}
