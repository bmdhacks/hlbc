class StdDowncast {
    public var tag:Int;

    public function new() {
        tag = 42;
    }

    static function describe(a:Dynamic):String {
        var d = Std.downcast(a, StdDowncast);
        if (d != null) {
            return "tag=" + d.tag;
        }
        return "not StdDowncast";
    }

    static function main() {
        var obj = new StdDowncast();
        Sys.println(describe(obj));
        Sys.println(describe("hello"));
    }
}
