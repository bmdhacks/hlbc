class ForLoop {
    static function main() {
        // Basic for loop with range
        var sum = 0;
        for (i in 0...1000) {
            sum += i;
        }
        Sys.println("sum=" + sum);  // 499500

        // For loop over array
        var arr = [10, 20, 30];
        var total = 0;
        for (x in arr) {
            total += x;
        }
        Sys.println("total=" + total);  // 60

        // Nested for loops
        var product = 0;
        for (i in 0...100) {
            for (j in 0...100) {
                product += i * j;
            }
        }
        Sys.println("product=" + product);
    }
}
