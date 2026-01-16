// Tests typed array decompilation
// Bug: decompiler outputs hl.types.ArrayObj instead of Array<T>
// Bug: array literal [] becomes NativeArray.alloc(...)

class TypedArrays {
    static function sumInts(arr:Array<Int>):Int {
        var sum = 0;
        for (x in arr) {
            sum += x;
        }
        return sum;
    }

    static function joinStrings(arr:Array<String>, sep:String):String {
        var result = "";
        var first = true;
        for (s in arr) {
            if (!first) {
                result += sep;
            }
            result += s;
            first = false;
        }
        return result;
    }

    static function makeRange(start:Int, end:Int):Array<Int> {
        var result:Array<Int> = [];
        var i = start;
        while (i < end) {
            result.push(i);
            i++;
        }
        return result;
    }

    static function filterPositive(arr:Array<Int>):Array<Int> {
        var result:Array<Int> = [];
        for (x in arr) {
            if (x > 0) {
                result.push(x);
            }
        }
        return result;
    }

    static function main() {
        var ints:Array<Int> = [1, 2, 3, 4, 5];
        Sys.println("sum=" + sumInts(ints));

        var strs:Array<String> = ["hello", "world", "test"];
        Sys.println("joined=" + joinStrings(strs, ", "));

        var range = makeRange(0, 5);
        Sys.println("range len=" + range.length);

        var mixed:Array<Int> = [-2, -1, 0, 1, 2, 3];
        var pos = filterPositive(mixed);
        Sys.println("positive count=" + pos.length);
    }
}
