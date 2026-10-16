package w71;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.instruction.ReturnInstruction;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.Instrumentation;

/** wp71: a java agent for the real vanilla 26.3 server: calls {@link W71Hook#afterTick} at the end of every ServerLevel.tick. */
public final class W71Agent {
    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new java.lang.instrument.ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> cls, java.security.ProtectionDomain pd, byte[] bytes) {
            if ("net/minecraft/world/entity/ai/util/LandRandomPos".equals(name)) {
                ClassFile cf = ClassFile.of();
                return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(
                        m -> m.methodName().equalsString("getPos") && m.methodType().equalsString("(Lnet/minecraft/world/entity/PathfinderMob;II)Lnet/minecraft/world/phys/Vec3;"),
                        (CodeBuilder b, CodeElement e) -> {
                            if (e instanceof java.lang.classfile.instruction.ReturnInstruction) {
                                b.dup();
                                b.invokestatic(ClassDesc.of("w71.W71Hook"), "landRet", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object));
                            }
                            b.with(e);
                        }));
            }
            if (!"net/minecraft/server/level/ServerLevel".equals(name)) return null;
            ClassFile cf = ClassFile.of();
            return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(
                    m -> m.methodName().equalsString("tick") && m.methodType().equalsString("(Ljava/util/function/BooleanSupplier;)V"),
                    (CodeBuilder b, CodeElement e) -> {
                        if (e instanceof ReturnInstruction) {
                            b.aload(0);
                            b.invokestatic(ClassDesc.of("w71.W71Hook"), "afterTick", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object));
                        }
                        b.with(e);
                    }));
            }
        });
    }
}
