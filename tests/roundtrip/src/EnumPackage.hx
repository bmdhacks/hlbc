import format.ImageType;

class EnumPackage {
    public function new() {}

    // Test enum switch with enum from another package
    public function processType(t:ImageType):String {
        return switch (t) {
            case NoImage: "none";
            case UncompressedColorMapped: "colormapped";
            case UncompressedTrueColor: "truecolor";
            case RLE: "rle";
        };
    }

    static function main() {
        var obj = new EnumPackage();
        Sys.println("NoImage -> " + obj.processType(ImageType.NoImage));
        Sys.println("RLE -> " + obj.processType(ImageType.RLE));
    }
}
