class Loop {
    public static function main() {
        var sum = 0;
        var i = 0;
        while (i < 5) {
            sum = sum + i;
            i = i + 1;
        }
        Sys.println("sum=" + sum);

        var count = 0;
        while (count < 3) {
            Sys.println("count=" + count);
            count = count + 1;
        }
    }
}
