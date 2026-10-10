static float4 frag_count;

struct SPIRV_Cross_Output
{
    float4 frag_count : SV_Target0;
};

void frag_main()
{
    frag_count = float4(1.0f, 0.0f, 0.0f, 1.0f);
}

SPIRV_Cross_Output main()
{
    frag_main();
    SPIRV_Cross_Output stage_output;
    stage_output.frag_count = frag_count;
    return stage_output;
}
