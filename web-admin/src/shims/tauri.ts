// The student app's desktop transport imports this; the console never runs in the desktop app.
export async function invoke<T>(_command: string, ..._args: unknown[]): Promise<T> {
  throw new Error("the admin console does not run in the desktop app");
}
