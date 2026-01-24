// Test: Switch statement with guard condition and nested control flow
// Based on hxsl/Flatten.hx mapExpr pattern

enum ExprType {
    TVar(v:Int);
    TConst(c:Int);
    TArray(e:Expr, idx:Expr);
    TOther;
}

class Expr {
    public var e:ExprType;
    public var t:Int;

    public function new(et:ExprType, typ:Int) {
        e = et;
        t = typ;
    }
}

class SwitchWithGuard {
    var varMap:Map<Int, Int>;

    public function new() {
        varMap = new Map();
        varMap.set(1, 10);
        varMap.set(2, 20);
    }

    // Pattern: switch with guard condition and nested control flow
    public function mapExpr(e:Expr):Expr {
        var result:Expr = switch (e.e) {
            case TVar(v):
                var a = varMap.get(v);
                if (a == null)
                    e
                else
                    new Expr(TConst(a), e.t);

            case TArray({ e: TVar(v) }, idx) if (!isConstInt(idx)):
                var a = varMap.get(v);
                if (a == null)
                    e
                else {
                    switch (e.t) {
                        case 1:
                            var stride = a >> 2;
                            if (stride == 0) throw "error";
                            new Expr(TConst(stride), e.t);
                        default:
                            throw "assert";
                    }
                }

            default:
                new Expr(e.e, e.t + 1);
        };
        return result;
    }

    function isConstInt(e:Expr):Bool {
        return switch (e.e) {
            case TConst(_): true;
            default: false;
        };
    }

    static function main() {
        var sg = new SwitchWithGuard();

        // Test TVar case with mapping
        var e1 = new Expr(TVar(1), 0);
        var r1 = sg.mapExpr(e1);
        trace("TVar mapped: " + getExprValue(r1));

        // Test TVar case without mapping
        var e2 = new Expr(TVar(99), 0);
        var r2 = sg.mapExpr(e2);
        trace("TVar unmapped: " + getExprValue(r2));

        // Test TArray case (should go to default since idx is const)
        var idx = new Expr(TConst(5), 0);
        var inner = new Expr(TVar(1), 0);
        var e3 = new Expr(TArray(inner, idx), 0);
        var r3 = sg.mapExpr(e3);
        trace("TArray const idx: " + getExprValue(r3));

        // Test default case
        var e4 = new Expr(TOther, 5);
        var r4 = sg.mapExpr(e4);
        trace("Other: " + r4.t);
    }

    static function getExprValue(e:Expr):String {
        return switch (e.e) {
            case TConst(v): "const(" + v + ")";
            case TVar(v): "var(" + v + ")";
            default: "other";
        };
    }
}
