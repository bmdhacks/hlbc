class LPt {
    public var x:Float;
    public var y:Float;

    public inline function new(x:Float, y:Float) {
        this.x = x;
        this.y = y;
    }

    public inline function equals(other:LPt):Bool {
        return x == other.x && y == other.y;
    }
}

class LNode {
    public var point:LPt;
    public var next:LNode;
    public var prev:LNode;
    public var value:Float;

    public function new(pt:LPt, v:Float) {
        point = pt;
        value = v;
    }
}

class LocatePointLike {
    var search:LNode;

    function new(s:LNode) {
        search = s;
    }

    // Full locatePoint equivalent:
    // 3-way branch + negated inline equals + nested if-else-if with inline equals
    // + while loops with inline equals break condition
    function locate(point:LPt):LNode {
        var px = point.x;
        var node = search;
        var nx = node.point.x;

        if (px == nx) {
            if (!point.equals(node.point)) {
                if (point.equals(node.prev.point)) {
                    node = node.prev;
                } else if (point.equals(node.next.point)) {
                    node = node.next;
                } else {
                    throw "not found";
                }
            }
        } else if (px < nx) {
            while ((node = node.prev) != null) {
                if (point.equals(node.point)) break;
            }
        } else {
            while ((node = node.next) != null) {
                if (point.equals(node.point)) break;
            }
        }

        if (node != null) search = node;
        return node;
    }

    static function main() {
        var p1 = new LPt(1.0, 10.0);
        var p2 = new LPt(2.0, 20.0);
        var p3 = new LPt(3.0, 30.0);
        var n1 = new LNode(p1, 1.0);
        var n2 = new LNode(p2, 2.0);
        var n3 = new LNode(p3, 3.0);
        n1.next = n2;
        n2.prev = n1;
        n2.next = n3;
        n3.prev = n2;

        var front = new LocatePointLike(n2);
        // exact match
        var r = front.locate(p2);
        Sys.println(r != null ? r.value : -1.0);
        // search backward
        r = front.locate(p1);
        Sys.println(r != null ? r.value : -1.0);
        // search forward
        r = front.locate(p3);
        Sys.println(r != null ? r.value : -1.0);
    }
}
