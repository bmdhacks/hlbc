// Minimal reproduction of TGA Reader patterns
// Tests: Property setter, VirtualClosure, Enum switch

enum ImageType {
    NoImage;
    UncompressedColorMapped;
    UncompressedTrueColor;
    RLE;
}

class TgaPatterns {
    public var input:haxe.io.BytesInput;

    public function new(data:haxe.io.Bytes) {
        input = new haxe.io.BytesInput(data);
        // Issue #1: Property setter - this compiles to CallMethod set_bigEndian
        input.bigEndian = false;
    }

    // Issue #2: Virtual method closure
    public function getReader():Void->Int {
        // This creates a VirtualClosure binding input.readByte to a function var
        var readFn:Void->Int = input.readByte;
        return readFn;
    }

    // Issue #3: Enum switch with enum-typed value
    public function processType(t:ImageType):String {
        return switch (t) {
            case NoImage: "none";
            case UncompressedColorMapped: "colormapped";
            case UncompressedTrueColor: "truecolor";
            case RLE: "rle";
        };
    }

    static function main() {
        var bytes = haxe.io.Bytes.alloc(16);
        var tga = new TgaPatterns(bytes);

        Sys.println("Testing property setter...");
        Sys.println("bigEndian = " + tga.input.bigEndian);

        Sys.println("Testing virtual closure...");
        var reader = tga.getReader();
        Sys.println("reader type: function");

        Sys.println("Testing enum switch...");
        Sys.println("NoImage -> " + tga.processType(ImageType.NoImage));
        Sys.println("RLE -> " + tga.processType(ImageType.RLE));
    }
}
