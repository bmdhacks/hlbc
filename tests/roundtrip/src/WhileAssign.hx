class WhileAssign {
    public var next:WhileAssign;
    public var value:Int;

    public function new(v:Int) {
        value = v;
    }

    static function countNodes(start:WhileAssign):Int {
        var count = 0;
        var node = start;
        while ((node = node.next) != null) {
            count++;
        }
        return count;
    }

    static function main() {
        var a = new WhileAssign(1);
        var b = new WhileAssign(2);
        var c = new WhileAssign(3);
        a.next = b;
        b.next = c;
        Sys.println(countNodes(a));
    }
}
