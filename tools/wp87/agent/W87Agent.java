package w87;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassTransform;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.Instrumentation;

/**
 * wp87: a java agent for the vanilla 26.3 server that logs which entity each side head of a wither picks as its target
 * (WitherBoss.setAlternativeTarget(head, entityId)), to $W87_OUT: "tick head entityType name".
 */
public final class W87Agent {
    static final ClassDesc HOOK = ClassDesc.of("w87.W87Hook");

    static CodeTransform atEntry(java.util.function.Consumer<CodeBuilder> inject) {
        return new CodeTransform() {
            @Override
            public void accept(CodeBuilder b, CodeElement e) {
                b.with(e);
            }

            @Override
            public void atStart(CodeBuilder b) {
                inject.accept(b);
            }
        };
    }

    public static void premain(String args, Instrumentation inst) {
        inst.addTransformer(new java.lang.instrument.ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> cls, java.security.ProtectionDomain pd, byte[] bytes) {
                try {
                    if (!"net/minecraft/world/entity/boss/wither/WitherBoss".equals(name)) {
                        return null;
                    }
                    ClassFile cf = ClassFile.of();
                    return cf.transformClass(cf.parse(bytes), ClassTransform.transformingMethodBodies(m -> m.methodName().equalsString("setAlternativeTarget"), atEntry(b -> {
                        b.aload(0);
                        b.iload(1);
                        b.iload(2);
                        b.invokestatic(HOOK, "alt", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object, ConstantDescs.CD_int, ConstantDescs.CD_int));
                    })));
                } catch (Throwable t) {
                    System.err.println("W87Agent: transform of " + name + " failed: " + t);
                    t.printStackTrace();
                    return null;
                }
            }
        });
    }
}
