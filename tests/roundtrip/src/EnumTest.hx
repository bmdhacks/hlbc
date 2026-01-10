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
        trace("color1=" + colorName(Color.Red));
        trace("color2=" + colorName(Color.Green));
        trace("color3=" + colorName(Color.Blue));

        trace("none=" + optionValue(Option.None));
        trace("some=" + optionValue(Option.Some(42)));
    }
}
