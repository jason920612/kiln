package w82;

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
 * wp82: a java agent for the vanilla 26.3 server that logs the end gateway's exit search: TheEndGatewayBlockEntity.isChunkEmpty(level, pos) -> boolean,
 * findValidSpawnInChunk(chunk) -> BlockPos and findTallestBlock(getter, center, dist, allowBedrock) -> BlockPos. Written to $W82_OUT.
 */
public final class W82Agent {
    static final ClassDesc HOOK = ClassDesc.of("w82.W82Hook");

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
                    if (!"net/minecraft/world/level/block/entity/TheEndGatewayBlockEntity".equals(name)) {
                        return null;
                    }
                    ClassFile cf = ClassFile.of();
                    bytes = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("isChunkEmpty"), onReturn(b -> {
                        b.dup();
                        b.aload(1);
                        b.invokestatic(HOOK, "chunkEmpty", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int, ConstantDescs.CD_Object));
                    })));
                    bytes = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("findValidSpawnInChunk"), onReturn(b -> {
                        b.dup();
                        b.aload(0);
                        b.invokestatic(HOOK, "validSpawn", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                    })));
                    bytes = cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("findTallestBlock"), onReturn(b -> {
                        b.dup();
                        b.aload(1);
                        b.invokestatic(HOOK, "tallest", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                    })));
                    return bytes;
                } catch (Throwable t) {
                    System.err.println("W82Agent: transform of " + name + " failed: " + t);
                    t.printStackTrace();
                    return null;
                }
            }
        });
    }
}
