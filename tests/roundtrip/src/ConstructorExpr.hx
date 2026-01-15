class Point {
    public var x:Int;
    public var y:Int;

    public function new(x:Int, y:Int) {
        this.x = x;
        this.y = y;
    }
}

class ConstructorExpr {
    static function computeX():Int {
        return 10;
    }

    static function computeY():Int {
        return 20;
    }

    static function main() {
        // Constructor with function call arguments
        var p1 = new Point(computeX(), computeY());
        Sys.println("p1: " + p1.x + "," + p1.y);

        // Constructor with expression arguments
        var a = 5;
        var b = 7;
        var p2 = new Point(a * 2, b + 3);
        Sys.println("p2: " + p2.x + "," + p2.y);

        // Nested constructor calls
        var p3 = new Point(new Point(1, 2).x, new Point(3, 4).y);
        Sys.println("p3: " + p3.x + "," + p3.y);
    }
}
