package w72;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.instruction.ReturnInstruction;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.Instrumentation;

/**
 * wp72: a java agent for the real vanilla 26.3 server that counts what natural spawning does with slimes:
 * every call of Slime.checkSlimeSpawnRules (its position, whether the position is in a slime chunk, the result) and every
 * NaturalSpawner.isValidSpawnPostion of a slime (the result). Written to $W72_OUT at exit and every 20000 ticks.
 */
public final class W72Agent {
    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new java.lang.instrument.ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> cls, java.security.ProtectionDomain pd, byte[] bytes) {
                if ("net/minecraft/world/entity/monster/cubemob/Slime".equals(name)) {
                    ClassFile cf = ClassFile.of();
                    return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(
                            m -> m.methodName().equalsString("checkSlimeSpawnRules"),
                            (CodeBuilder b, CodeElement e) -> {
                                if (e instanceof ReturnInstruction) {
                                    b.dup();
                                    b.aload(3);
                                    b.aload(1);
                                    b.invokestatic(ClassDesc.of("w72.W72Hook"), "slimeRet", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                                }
                                b.with(e);
                            }));
                }
                return null;
            }
        });
    }
}
