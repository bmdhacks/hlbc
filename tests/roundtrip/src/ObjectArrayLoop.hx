// Test for-in loops over object arrays
// This exercises the ArrayObj .array field access pattern which requires
// proper SSA versioning to avoid undefined variable references.

class Item {
    public var value:Int;
    public function new(v:Int) {
        this.value = v;
    }
    public function getValue():Int {
        return value;
    }
    public function process():Void {
        trace("processing " + value);
    }
}

class ObjectArrayLoop {
    var items:Array<Item>;

    public function new() {
        items = new Array<Item>();
        items.push(new Item(10));
        items.push(new Item(20));
        items.push(new Item(30));
    }

    // Instance method iterating over object array field
    // This pattern triggered the SSA versioning bug where array_bytes_source
    // stored a raw register instead of the SSA-versioned expression
    public function sumValues():Int {
        var sum = 0;
        for (item in items) {
            sum += item.getValue();
        }
        return sum;
    }

    // Another iteration pattern with method calls on each object
    public function processAll():Void {
        for (item in items) {
            item.process();
        }
    }

    static function main() {
        // Test with local object array
        var arr = new Array<Item>();
        arr.push(new Item(1));
        arr.push(new Item(2));
        arr.push(new Item(3));

        var total = 0;
        for (obj in arr) {
            total += obj.value;
        }
        trace("local total=" + total);  // 6

        // Test with instance field array
        var loop = new ObjectArrayLoop();
        var sum = loop.sumValues();
        trace("instance sum=" + sum);  // 60

        loop.processAll();
    }
}
