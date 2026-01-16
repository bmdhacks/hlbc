class Closure {
    static function applyTwice(f:Int->Int, x:Int):Int {
        return f(f(x));
    }

    static function makeAdder(n:Int):Int->Int {
        return function(x:Int):Int {
            return x + n;
        };
    }

    static function makeMultiplier(n:Int):Int->Int {
        return function(x:Int):Int {
            return x * n;
        };
    }

    static function main() {
        // Inline lambda
        var double = function(x:Int):Int { return x * 2; };
        Sys.println("double(5)=" + double(5));
        Sys.println("doubled twice=" + applyTwice(double, 3));

        // Closure capturing variable
        var add5 = makeAdder(5);
        Sys.println("add5(10)=" + add5(10));

        var add10 = makeAdder(10);
        Sys.println("add10(7)=" + add10(7));

        // Another closure
        var mult3 = makeMultiplier(3);
        Sys.println("mult3(4)=" + mult3(4));
    }
}
