enum Color {
    Red;
    Green;
    Blue;
}

enum Option<T> {
    None;
    Some(value:T);
}

class EnumTest {
    static function colorName(c:Color):String {
        return switch (c) {
            case Red: "red";
            case Green: "green";
            case Blue: "blue";
        };
    }

    static function optionValue(opt:Option<Int>):Int {
        return switch (opt) {
            case None: -1;
            case Some(v): v;
        };
    }

    static function main() {
        Sys.println("color1=" + colorName(Color.Red));
        Sys.println("color2=" + colorName(Color.Green));
        Sys.println("color3=" + colorName(Color.Blue));

        Sys.println("none=" + optionValue(Option.None));
        Sys.println("some=" + optionValue(Option.Some(42)));
    }
}
