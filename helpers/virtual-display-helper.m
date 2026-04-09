#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#import <objc/message.h>

typedef id   (*MsgSend_id_id)(id, SEL, id);
typedef id   (*MsgSend_id_NSInt_NSInt_dbl)(id, SEL, NSInteger, NSInteger, double);
typedef void (*MsgSend_void_id)(id, SEL, id);
typedef uint32_t (*MsgSend_u32)(id, SEL);

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc < 3) {
            fprintf(stderr, "Usage: virtual-display-helper <width> <height> [refresh_rate]\n");
            return 1;
        }

        NSInteger width  = atoi(argv[1]);
        NSInteger height = atoi(argv[2]);
        double refreshRate = argc > 3 ? atof(argv[3]) : 60.0;

        Class DescClass     = NSClassFromString(@"CGVirtualDisplayDescriptor");
        Class ModeClass     = NSClassFromString(@"CGVirtualDisplayMode");
        Class SettingsClass = NSClassFromString(@"CGVirtualDisplaySettings");
        Class DisplayClass  = NSClassFromString(@"CGVirtualDisplay");

        if (!DescClass || !ModeClass || !SettingsClass || !DisplayClass) {
            fprintf(stderr, "ERROR: CGVirtualDisplay API not available (requires macOS 14+)\n");
            return 1;
        }

        id desc = [[DescClass alloc] init];
        [desc setValue:@"Desk Switch Virtual Display" forKey:@"name"];
        [desc setValue:@(width)  forKey:@"maxPixelsWide"];
        [desc setValue:@(height) forKey:@"maxPixelsHigh"];
        [desc setValue:[NSValue valueWithSize:NSMakeSize(530, 300)] forKey:@"sizeInMillimeters"];
        [desc setValue:@(0xDEAD) forKey:@"productID"];
        [desc setValue:@(0xBEEF) forKey:@"vendorID"];
        [desc setValue:@(12345)  forKey:@"serialNum"];

        id mode = ((MsgSend_id_NSInt_NSInt_dbl)objc_msgSend)(
            [ModeClass alloc],
            sel_registerName("initWithWidth:height:refreshRate:"),
            width, height, refreshRate
        );
        if (!mode) {
            fprintf(stderr, "ERROR: Failed to create virtual display mode %ldx%ld@%.0fHz\n",
                    (long)width, (long)height, refreshRate);
            return 1;
        }

        id settings = [[SettingsClass alloc] init];
        [settings setValue:@[mode] forKey:@"modes"];

        id display = ((MsgSend_id_id)objc_msgSend)(
            [DisplayClass alloc],
            sel_registerName("initWithDescriptor:"),
            desc
        );
        if (!display) {
            fprintf(stderr, "ERROR: Failed to create virtual display\n");
            return 1;
        }

        ((MsgSend_void_id)objc_msgSend)(
            display,
            sel_registerName("applySettings:"),
            settings
        );

        uint32_t displayID = ((MsgSend_u32)objc_msgSend)(
            display,
            sel_registerName("displayID")
        );

        if (displayID == 0) {
            fprintf(stderr, "ERROR: Virtual display created but has ID 0 — may not be functional\n");
        }

        fprintf(stdout, "DISPLAY_ID=%u\n", displayID);
        fflush(stdout);

        char buf[256];
        while (fgets(buf, sizeof(buf), stdin) != NULL) {
            if (strncmp(buf, "QUIT", 4) == 0) break;
        }

        return 0;
    }
}
