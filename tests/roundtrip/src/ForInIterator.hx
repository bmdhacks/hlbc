class ForInIterator {
    static function main() {
        // StringMap iterator uses hasNext()/next() - not inlined
        var map = new haxe.ds.StringMap<Int>();
        map.set("a", 1);
        map.set("b", 2);
        map.set("c", 3);

        var sum = 0;
        for (key in map.keys()) {
            var val = map.get(key);
            if (val != null) {
                sum += val;
            }
        }
        Sys.println("sum=" + sum);
    }
}
