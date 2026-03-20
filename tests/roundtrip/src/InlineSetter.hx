class InlineSetter {
    public var x:Float;
    public var y:Float;
    public var dirty:Bool;

    public function new() {
        x = 0;
        y = 0;
        dirty = false;
    }

    public inline function setX(v:Float) {
        if (v != x) {
            x = v;
            dirty = true;
        }
    }

    public inline function setY(v:Float) {
        if (v != y) {
            y = v;
            dirty = true;
        }
    }

    public inline function setPos(nx:Float, ny:Float) {
        setX(nx);
        setY(ny);
    }

    static function animate(obj:InlineSetter, steps:Int):Int {
        var dirtyCount = 0;
        for (i in 0...steps) {
            obj.setPos(i * 1.5, i * 2.5);
            if (obj.dirty) {
                dirtyCount++;
                obj.dirty = false;
            }
        }
        return dirtyCount;
    }

    static function main() {
        var obj = new InlineSetter();
        Sys.println(animate(obj, 10));
    }
}
