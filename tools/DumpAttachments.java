// Prints the PASSENGER attachment point (`EntityAttachments.getAverage(PASSENGER)`: height, offset along the body)
// of every entity type, adult and (where the type scales) baby: where shorn equipment drops from.
//
// usage: java -cp <server jar + libraries> tools/DumpAttachments.java

import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityAttachment;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.phys.Vec3;

public class DumpAttachments {
    public static void main(String[] args) {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        for (EntityType<?> t : BuiltInRegistries.ENTITY_TYPE) {
            Vec3 v = t.getDimensions().attachments().getAverage(EntityAttachment.PASSENGER);
            System.out.println(BuiltInRegistries.ENTITY_TYPE.getKey(t) + " " + v.x + " " + v.y + " " + v.z + " h=" + t.getDimensions().height());
        }
    }
}
