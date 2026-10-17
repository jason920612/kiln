package w72;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.classfile.instruction.ReturnInstruction;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.Instrumentation;

/**
 * wp72: a java agent for the real vanilla 26.3 server that counts what natural spawning does:
 * Slime.checkSlimeSpawnRules (position, slime chunk, result), NaturalSpawner.getRandomSpawnMobAt (the type picked),
 * NaturalSpawner.isValidSpawnPostitionForType (type, result) and NaturalSpawner.isValidPositionForMob (type, result).
 * Written to $W72_OUT at exit.
 */
public final class W72Agent {
    static final ClassDesc HOOK = ClassDesc.of("w72.W72Hook");

    static CodeTransform onReturn(java.util.function.Consumer<CodeBuilder> before) {
        return (CodeBuilder b, CodeElement e) -> {
            if (e instanceof ReturnInstruction) {
                before.accept(b);
            }
            b.with(e);
        };
    }

    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new java.lang.instrument.ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> cls, java.security.ProtectionDomain pd, byte[] bytes) {
                try {
                    return transform0(name, bytes);
                } catch (Throwable t) {
                    System.err.println("W72Agent: transform of " + name + " failed: " + t);
                    t.printStackTrace();
                    return null;
                }
            }

            byte[] transform0(String name, byte[] bytes) {
                return apply(name, bytes);
            }
        });
    }

    /** The transformed class bytes, or null for a class the agent leaves alone. */
    public static byte[] apply(String name, byte[] bytes) {
        {
            {
                ClassFile cf = ClassFile.of();
                if ("net/minecraft/world/entity/monster/cubemob/Slime".equals(name)) {
                    return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(
                            m -> m.methodName().equalsString("checkSlimeSpawnRules"),
                            onReturn(b -> {
                                b.dup();
                                b.aload(3);
                                b.aload(1);
                                b.invokestatic(HOOK, "slimeRet", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                            })));
                }
                if ("net/minecraft/world/level/NaturalSpawner$SpawnState".equals(name)) {
                    return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("canSpawnForCategoryGlobal"), onReturn(b -> {
                        b.dup();
                        b.aload(0);
                        b.aload(1);
                        b.invokestatic(HOOK, "globalCap", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                    })));
                }
                if ("net/minecraft/world/level/NaturalSpawner".equals(name)) {
                    bytes = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("getFilteredSpawningCategories"), onReturn(b -> {
                        b.dup();
                        b.invokestatic(HOOK, "cats", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object));
                    })));
                    bytes = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("spawnCategoryForChunk"), onReturn(b -> {
                        b.aload(0);
                        b.aload(2);
                        b.invokestatic(HOOK, "chunkAttempt", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                    })));
                    byte[] b1 = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("isValidSpawnPostitionForType"), onReturn(b -> {
                        b.dup();
                        b.aload(4);
                        b.aload(5);
                        b.invokestatic(HOOK, "validPos", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                    })));
                    byte[] b2 = cf.transformClass(cf.parse(b1), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("isValidPositionForMob"), onReturn(b -> {
                        b.dup();
                        b.aload(1);
                        b.invokestatic(HOOK, "validMob", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object));
                    })));
                    return cf.transformClass(cf.parse(b2), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("getRandomSpawnMobAt"), onReturn(b -> {
                        b.dup();
                        b.invokestatic(HOOK, "pick", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object));
                    })));
                }
                return null;
            }
        }
    }

    public static void main(String[] a) throws Exception {
        var z = new java.util.zip.ZipFile(a[0]);
        for (String n : new String[] {"net/minecraft/world/level/NaturalSpawner", "net/minecraft/world/entity/monster/cubemob/Slime"}) {
            byte[] in = z.getInputStream(z.getEntry(n + ".class")).readAllBytes();
            byte[] out = apply(n, in);
            System.out.println(n + ": " + in.length + " -> " + (out == null ? "null" : out.length + (new String(out, java.nio.charset.StandardCharsets.ISO_8859_1).contains("validPos") ? " has validPos" : " NO validPos")));
        }
    }
}
