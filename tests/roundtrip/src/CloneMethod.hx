class Inner {
    public var value:Int;

    public function new(v:Int) {
        value = v;
    }

    public function clone():Inner {
        return new Inner(value);
    }
}

class CloneMethod {
    public var inner:Inner;
    public var count:Int;

    public function new(i:Inner, c:Int) {
        inner = i;
        count = c;
    }

    public function clone():CloneMethod {
        return new CloneMethod(inner.clone(), count);
    }

    public static function main() {
        var orig = new CloneMethod(new Inner(42), 10);
        var copy = orig.clone();
        trace(copy.inner.value);
        trace(copy.count);
    }
}
