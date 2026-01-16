import pkg.BaseClass;

// Tests that inheritance from a packaged class preserves the full path
// Bug: decompiler outputs "extends BaseClass" instead of "extends pkg.BaseClass"
// which causes "extends PackageInheritance" self-reference error

class PackageInheritance extends BaseClass {
    var extra:String;

    public function new(v:Int, e:String) {
        super(v);
        this.extra = e;
    }

    override public function describe():String {
        return super.describe() + " with " + extra;
    }

    public static function main() {
        var obj = new PackageInheritance(42, "extras");
        Sys.println(obj.describe());
        Sys.println("value=" + obj.getValue());
    }
}
