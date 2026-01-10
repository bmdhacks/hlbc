class SimpleClass {
    var value:Int;

    public function new(v:Int) {
        this.value = v;
    }

    public function getValue():Int {
        return this.value;
    }

    public function setValue(v:Int):Void {
        this.value = v;
    }

    public function doubleValue():Int {
        return this.value * 2;
    }

    static function main() {
        var obj = new SimpleClass(10);
        trace("initial=" + obj.getValue());

        obj.setValue(25);
        trace("after set=" + obj.getValue());

        trace("doubled=" + obj.doubleValue());
    }
}
