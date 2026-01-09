class Conditionals {
    public static function main() {
        var x = 5;
        if (x > 3) {
            trace("x is greater than 3");
        } else {
            trace("x is not greater than 3");
        }

        var y = 10;
        if (y == 10) {
            trace("y equals 10");
        }

        var z = x > 0 ? "positive" : "non-positive";
        trace("z=" + z);
    }
}
