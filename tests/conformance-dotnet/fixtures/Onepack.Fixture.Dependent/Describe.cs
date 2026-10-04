namespace Onepack.Fixture.Dependent
{
    public static class Describe
    {
        public static string Message() => "dependent -> " + Onepack.Fixture.Basic.Greeting.Hello();
    }
}
