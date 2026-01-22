// Test pattern: many OR conditions leading to single throw
// This pattern appears in hxd.BitmapData.drawScaled
// Pattern: if (a || b || c || d || e || f || g || h) throw "error";
class ManyOrThrow {
    static function validateBounds(x:Int, y:Int, w:Int, h:Int, srcX:Int, srcY:Int, srcW:Int, srcH:Int):Void {
        if (x < 0 || y < 0 || w < 0 || h < 0 || srcX < 0 || srcY < 0 || srcW < 0 || srcH < 0)
            throw "Outside bounds";
        Sys.println("Bounds valid");
    }

    static function main() {
        try {
            validateBounds(0, 0, 10, 10, 0, 0, 10, 10);
            validateBounds(-1, 0, 10, 10, 0, 0, 10, 10);
        } catch (e:Dynamic) {
            Sys.println("Caught: " + e);
        }
    }
}
