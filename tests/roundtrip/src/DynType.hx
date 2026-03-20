class DynType {
    static function getTypeName(x:Dynamic):String {
        var t:hl.Type = untyped $tdyntype(x);
        var kind:Int = untyped $tkind(t);
        if (kind == 0)
            return "void";
        else if (kind == 1)
            return "u8";
        else if (kind == 2)
            return "u16";
        else if (kind == 3)
            return "i32";
        else
            return "other";
    }

    static function main() {
        Sys.println(getTypeName(42));
        Sys.println(getTypeName(true));
        Sys.println(getTypeName("hello"));
    }
}
