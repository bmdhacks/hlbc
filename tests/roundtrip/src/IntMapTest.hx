import haxe.ds.IntMap;

class IntMapTest {
    static function main() {
        var map = new IntMap<String>();
        map.set(1, "one");
        map.set(2, "two");
        map.set(3, "three");

        // Iterating over IntMap triggers virtual interface caching
        var total = 0;
        for (key in map.keys()) {
            total += key;
        }
        trace("keys total=" + total);  // 1+2+3 = 6

        // Also iterate over values
        var values = "";
        for (val in map) {
            values += val + ",";
        }
        trace("values=" + values);
    }
}
