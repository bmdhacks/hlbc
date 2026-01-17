class DeepIfChain {
    // This should create deeply nested if-else due to null checks and length comparisons
    public static function create(id:Null<String>):Int {
        // Adding null checks and complex conditions to force preambles
        if (id != null && id.length == 1 && id == "a") return 1;
        if (id != null && id.length == 1 && id == "b") return 2;
        if (id != null && id.length == 1 && id == "c") return 3;
        if (id != null && id.length == 1 && id == "d") return 4;
        if (id != null && id.length == 1 && id == "e") return 5;
        if (id != null && id.length == 1 && id == "f") return 6;
        if (id != null && id.length == 1 && id == "g") return 7;
        if (id != null && id.length == 1 && id == "h") return 8;
        if (id != null && id.length == 1 && id == "i") return 9;
        if (id != null && id.length == 1 && id == "j") return 10;
        if (id != null && id.length == 2 && id == "aa") return 11;
        if (id != null && id.length == 2 && id == "bb") return 12;
        if (id != null && id.length == 2 && id == "cc") return 13;
        if (id != null && id.length == 2 && id == "dd") return 14;
        if (id != null && id.length == 2 && id == "ee") return 15;
        if (id != null && id.length == 2 && id == "ff") return 16;
        if (id != null && id.length == 2 && id == "gg") return 17;
        if (id != null && id.length == 2 && id == "hh") return 18;
        if (id != null && id.length == 2 && id == "ii") return 19;
        if (id != null && id.length == 2 && id == "jj") return 20;
        return 0;
    }

    public static function main() {
        trace(create("a"));
        trace(create("e"));
        trace(create("aa"));
        trace(create("jj"));
        trace(create(null));
        trace(create("?"));
    }
}
