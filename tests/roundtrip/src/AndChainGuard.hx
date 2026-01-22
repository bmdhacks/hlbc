// This test mimics the exact structure of hxd.BitmapData.setPixel line 595:
//   if( x >= 0 && y >= 0 && x < data.width && y < data.height ) data.pixels[...] = c;
//
// The compiler transforms this AND chain into multiple conditional jumps
// that all jump to the end if ANY condition fails.

class InnerData {
    public var width:Int;
    public var height:Int;
    public var pixels:haxe.io.Bytes;

    public function new(w:Int, h:Int) {
        width = w;
        height = h;
        pixels = haxe.io.Bytes.alloc(w * h * 4);
    }
}

class AndChainGuard {
    var data:InnerData;

    public function new(w:Int, h:Int) {
        data = new InnerData(w, h);
    }

    // This should produce the same CFG as hxd.BitmapData.setPixel:
    // if (x >= 0 && y >= 0 && x < width && y < height) do_work;
    // Compiles to:
    // - JSLt x < 0 -> end (skip work)
    // - JSLt y < 0 -> end (skip work)
    // - JSGte x >= width -> end (skip work)
    // - JSGte y >= height -> end (skip work)
    // - do work
    // - end
    public function setPixel(x:Int, y:Int, color:Int):Void {
        if (x >= 0 && y >= 0 && x < data.width && y < data.height)
            data.pixels.setInt32((y * data.width + x) * 4, color);
    }

    public static function main() {
        var bmp = new AndChainGuard(100, 100);

        // Test valid pixels
        bmp.setPixel(50, 50, 0xFF0000);
        bmp.setPixel(0, 0, 0x00FF00);
        bmp.setPixel(99, 99, 0x0000FF);

        // Test invalid pixels (should skip the work)
        bmp.setPixel(-1, 50, 0xFFFFFF);
        bmp.setPixel(50, -1, 0xFFFFFF);
        bmp.setPixel(100, 50, 0xFFFFFF);
        bmp.setPixel(50, 100, 0xFFFFFF);

        Sys.println("AndChainGuard test completed");
    }
}
