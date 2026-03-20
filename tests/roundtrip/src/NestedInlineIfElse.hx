class Pt {
    public var x:Float;
    public var y:Float;

    public inline function new(x:Float, y:Float) {
        this.x = x;
        this.y = y;
    }

    public inline function equals(other:Pt):Bool {
        return x == other.x && y == other.y;
    }
}

class NestedInlineIfElse {
    // Matches locatePoint's px==nx branch:
    // if (!equals(a)) { if (equals(b)) ... else if (equals(c)) ... else throw }
    static function matchOrThrow(target:Pt, a:Pt, b:Pt, c:Pt):String {
        if (!target.equals(a)) {
            if (target.equals(b)) {
                return "b";
            } else if (target.equals(c)) {
                return "c";
            } else {
                throw "no match";
            }
        }
        return "a";
    }

    static function main() {
        var t1 = new Pt(1.0, 2.0);
        var t2 = new Pt(3.0, 4.0);
        var t3 = new Pt(5.0, 6.0);
        Sys.println(matchOrThrow(t1, t1, t2, t3));
        Sys.println(matchOrThrow(t2, t1, t2, t3));
        Sys.println(matchOrThrow(t3, t1, t2, t3));
    }
}
