private enum Token {
    TInt(v: Int);
    TString(v: String);
    TEnd;
}

class PrivateEnum {
    var current: Token;

    public function new() {
        current = TEnd;
    }

    public function setInt(v: Int) {
        current = TInt(v);
    }

    public function setString(v: String) {
        current = TString(v);
    }

    public function describe(): String {
        return switch (current) {
            case TInt(v): "int:" + v;
            case TString(v): "string:" + v;
            case TEnd: "end";
        };
    }

    public static function main() {
        var p = new PrivateEnum();
        trace(p.describe());
        p.setInt(42);
        trace(p.describe());
        p.setString("hello");
        trace(p.describe());
    }
}
