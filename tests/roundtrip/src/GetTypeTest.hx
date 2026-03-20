class GetTypeTest {
    static function describeType(value:Dynamic):String {
        var t = Type.typeof(value);
        switch (t) {
            case TInt:
                return "int";
            case TFloat:
                return "float";
            case TBool:
                return "bool";
            case TNull:
                return "null";
            case TClass(c):
                return "class:" + Type.getClassName(c);
            default:
                return "other";
        }
    }

    static function main() {
        Sys.println(describeType(42));
        Sys.println(describeType(3.14));
        Sys.println(describeType(true));
        Sys.println(describeType(null));
        Sys.println(describeType("hello"));
    }
}
