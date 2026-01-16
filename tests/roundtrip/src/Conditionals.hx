class Conditionals {
    public static function main() {
        var x = 5;
        if (x > 3) {
            Sys.println("x is greater than 3");
        } else {
            Sys.println("x is not greater than 3");
        }

        var y = 10;
        if (y == 10) {
            Sys.println("y equals 10");
        }

        var z = x > 0 ? "positive" : "non-positive";
        Sys.println("z=" + z);
    }
}
