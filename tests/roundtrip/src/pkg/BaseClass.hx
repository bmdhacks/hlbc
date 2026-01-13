package pkg;

class BaseClass {
    var value:Int;

    public function new(v:Int) {
        this.value = v;
    }

    public function getValue():Int {
        return value;
    }

    public function describe():String {
        return "BaseClass(" + value + ")";
    }
}
