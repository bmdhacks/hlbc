class Vec2 {
    public var x:Float;
    public var y:Float;

    public inline function new(x:Float, y:Float) {
        this.x = x;
        this.y = y;
    }

    public inline function equals(other:Vec2):Bool {
        return x == other.x && y == other.y;
    }

    public inline function distSq(other:Vec2):Float {
        var dx = x - other.x;
        var dy = y - other.y;
        return dx * dx + dy * dy;
    }
}

class InlineAndChain {
    static function findMatch(points:Array<Vec2>, target:Vec2):Int {
        for (i in 0...points.length) {
            if (points[i].equals(target)) {
                return i;
            }
        }
        return -1;
    }

    static function closest(a:Vec2, b:Vec2, c:Vec2, target:Vec2):String {
        var da = a.distSq(target);
        var db = b.distSq(target);
        var dc = c.distSq(target);
        if (da < db && da < dc) {
            return "a";
        } else if (db < dc) {
            return "b";
        } else {
            return "c";
        }
    }

    static function main() {
        var p1 = new Vec2(1.0, 2.0);
        var p2 = new Vec2(3.0, 4.0);
        var p3 = new Vec2(5.0, 6.0);
        var target = new Vec2(3.0, 4.0);
        var arr = [p1, p2, p3];
        Sys.println(findMatch(arr, target));
        Sys.println(closest(p1, p2, p3, new Vec2(4.5, 5.5)));
    }
}
