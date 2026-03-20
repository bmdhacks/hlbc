class InlineNullCheck {
    public var name:String;
    public var score:Int;

    public function new(n:String, s:Int) {
        name = n;
        score = s;
    }

    static inline function nameOrDefault(obj:InlineNullCheck):String {
        return obj != null ? obj.name : "unknown";
    }

    static inline function scoreOrZero(obj:InlineNullCheck):Int {
        return obj != null ? obj.score : 0;
    }

    static function summarize(items:Array<InlineNullCheck>):String {
        var total = 0;
        var names = "";
        for (i in 0...items.length) {
            var item = items[i];
            total += scoreOrZero(item);
            var n = nameOrDefault(item);
            if (names.length > 0) names += ",";
            names += n;
        }
        return names + "=" + total;
    }

    static function main() {
        var arr:Array<InlineNullCheck> = [
            new InlineNullCheck("alice", 10),
            null,
            new InlineNullCheck("bob", 20)
        ];
        Sys.println(summarize(arr));
    }
}
