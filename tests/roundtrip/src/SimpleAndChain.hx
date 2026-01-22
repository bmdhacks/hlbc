// Simple AND chain test - no nesting
class SimpleAndChain {
    var w:Int;
    var h:Int;

    public function new(width:Int, height:Int) {
        w = width;
        h = height;
    }

    // Simple 4-condition AND chain (like hxd.BitmapData.setPixel)
    public function inBounds(x:Int, y:Int):Bool {
        if (x >= 0 && y >= 0 && x < w && y < h)
            return true;
        return false;
    }

    // Simple 2-condition AND chain
    public function inBoundsX(x:Int):Bool {
        if (x >= 0 && x < w)
            return true;
        return false;
    }

    public static function main() {
        var s = new SimpleAndChain(100, 100);
        Sys.println("inBounds(50,50)=" + s.inBounds(50, 50));
        Sys.println("inBounds(-1,50)=" + s.inBounds(-1, 50));
        Sys.println("inBounds(100,50)=" + s.inBounds(100, 50));
        Sys.println("inBoundsX(50)=" + s.inBoundsX(50));
        Sys.println("inBoundsX(-1)=" + s.inBoundsX(-1));
    }
}
