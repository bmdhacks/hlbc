class LongIfChain {
    public static function create(id:String):Int {
        if (id == "a") return 1;
        else if (id == "b") return 2;
        else if (id == "c") return 3;
        else if (id == "d") return 4;
        else if (id == "e") return 5;
        else if (id == "f") return 6;
        else if (id == "g") return 7;
        else if (id == "h") return 8;
        else if (id == "i") return 9;
        else if (id == "j") return 10;
        else if (id == "k") return 11;
        else if (id == "l") return 12;
        else if (id == "m") return 13;
        else if (id == "n") return 14;
        else if (id == "o") return 15;
        else if (id == "p") return 16;
        else if (id == "q") return 17;
        else if (id == "r") return 18;
        else if (id == "s") return 19;
        else if (id == "t") return 20;
        else if (id == "u") return 21;
        else if (id == "v") return 22;
        else if (id == "w") return 23;
        else if (id == "x") return 24;
        else if (id == "y") return 25;
        else if (id == "z") return 26;
        else return 0;
    }

    public static function main() {
        trace(create("a"));
        trace(create("m"));
        trace(create("z"));
        trace(create("?"));
    }
}
