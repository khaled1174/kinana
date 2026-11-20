import { cn, type ClassValue } from "cn";

// The registry files use only simple variant maps. Keep them local and merge
// with the approved cn package; do not add another runtime dependency.
type Choices = Record<string, Record<string, ClassValue>>;
type Props<V extends Choices> = { [K in keyof V]?: keyof V[K] | null } & { className?: ClassValue };
export function variants<const V extends Choices>(base: ClassValue, config: { variants: V; defaultVariants?: Props<V> }) {
  return (props: Props<V> = {}) => cn(base, ...Object.keys(config.variants).map(key => {
    const value = props[key] ?? config.defaultVariants?.[key];
    return value == null ? undefined : config.variants[key][String(value)];
  }), props.className);
}
export type VariantProps<T extends (...args: never[]) => unknown> = Omit<NonNullable<Parameters<T>[0]>, "className">;
