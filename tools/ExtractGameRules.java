// Dumps every game rule's default value from the vanilla 26.3 registry.
// Run through `cargo xtask extract`; output: a JSON array in registry order.
//
// usage: java -cp <server jar + libraries> tools/ExtractGameRules.java <out.json>

import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.gamerules.GameRule;

public class ExtractGameRules {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            out.println("[");
            var rules = BuiltInRegistries.GAME_RULE;
            int i = 0, n = rules.size();
            for (GameRule<?> rule : rules) {
                Object value = rule.defaultValue();
                String kind = value instanceof Boolean ? "bool" : value instanceof Integer ? "int" : "other";
                out.printf("{\"name\":\"%s\",\"type\":\"%s\",\"default\":\"%s\"}%s%n",
                        rules.getKey(rule), kind, value, ++i < n ? "," : "");
            }
            out.println("]");
        }
    }
}
